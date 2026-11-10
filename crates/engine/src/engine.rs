//! The Git-aware projection pipeline shared by every frontend.
//!
//! The engine composes read-only Git snapshot reads with the pure core
//! rendering functions. It owns no long-lived cache: each operation builds and
//! drops its own blob and projection caches, so memory is bounded by one call.
//!
//! Projection is deterministic: the language adapters are consulted in a fixed
//! order, and file lists come back in raw repository path-byte order. A
//! reserved snapshot specifier starts with `:`, which no Git refname can
//! contain, so a commit is never mistaken for a snapshot; with merge-base the
//! base becomes `merge-base(BASE, TARGET)` and both sides must be commits.
//!
//! A worktree side is the tracked index paths unioned with untracked,
//! non-ignored files, and membership is authoritative: a path outside that set
//! is absent even when a file exists on disk, so an ignored file never leaks
//! into a diff. A focused diff keeps only paths whose canonical old and new
//! projections differ, so an implementation-only change produces nothing. The
//! one exception is a detected move: it remains visible when both projections
//! are equal and nonempty, and is omitted only when both requested-mode
//! projections are empty. [`project_source`] is the editor-facing entry point:
//! it never touches Git, and an unsupported path is a typed
//! [`EngineError::UnsupportedPath`] rather than a silent empty result.
//!
//! Projections are cached per operation under `(blob id, path, mode)`. The path
//! is part of the key because several languages namespace `stable_key` by
//! repository path, so items projected for one path are invalid for a different
//! path that shares a blob.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use base::{
    AreaSet, DiagnosticContext, FileDiff, FileOutline, FileOutlineDiff, Language,
    LanguageProjector, Location, PathScope, PreparedSource, ProfileBudget, ProjectedFile,
    ProjectedItem, ProjectionError, ProjectionInput, ProjectionMode, RepoPath, Selection,
    SelectionGroup, SimilarityInterner, SimilarityOutcome, SourceOutline, SupportedPath, UnitMode,
    assemble_outline, decode_source, select_projector, source_similarity_at_least,
    source_similarity_at_least_prepared,
};
use git::{CommitStep, GitRepository, ObjectId, Revision, SnapshotRepository, SourceEntry};
use lang_elm::ElmProjector;
use lang_haskell::HaskellProjector;
use lang_python::PythonProjector;
use lang_rust::RustProjector;

use crate::config;
use crate::error::EngineError;

static ELM_PROJECTOR: ElmProjector = ElmProjector;
static HASKELL_PROJECTOR: HaskellProjector = HaskellProjector;
static PYTHON_PROJECTOR: PythonProjector = PythonProjector;
static RUST_PROJECTOR: RustProjector = RustProjector;

const PROJECTORS: [&dyn LanguageProjector; 4] = [
    &ELM_PROJECTOR,
    &HASKELL_PROJECTOR,
    &PYTHON_PROJECTOR,
    &RUST_PROJECTOR,
];

// The canonical Git empty-tree object ids accepted as `:empty`, for SHA-1 and
// SHA-256 object formats.
const EMPTY_TREE_IDS: [&str; 2] = [
    "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
    "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321",
];

// Move detection bounds. Fuzzy candidates are read only when both sides are
// within the byte cap; the cap is applied from metadata before the read.
const FUZZY_MAX_BYTES: u64 = 256 * 1024;
const FUZZY_MAX_UNPAIRED_PER_SIDE: usize = 256;
const FUZZY_MAX_ELIGIBLE_PAIRS: usize = 4096;
const MOVE_SIMILARITY_THRESHOLD: u8 = 50;
// Worktree bytes read during detection are retained only when they can be
// reused for a selected projection. This bound is independent of the number of
// candidates; the cache never evicts, so a decision never depends on eviction.
const WORKTREE_CACHE_BUDGET: usize = 32 * 1024 * 1024;
const WORKTREE_CACHE_MAX_FILE: usize = FUZZY_MAX_BYTES as usize;
// At most 512 fuzzy candidates can exist (256 per side), each capped at
// 256 KiB. The frozen fuzzy snapshot is therefore bounded by 128 MiB and is
// separate from the 32 MiB exact-only worktree cache.
const FUZZY_SNAPSHOT_MAX_BYTES: usize = 128 * 1024 * 1024;

// A per-file consistency guard. It is checked around every worktree read or
// hash; a changed file is rejected rather than mixing two versions.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    mtime: i64,
    #[cfg(unix)]
    mtime_nsec: i64,
}

impl FileFingerprint {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                dev: metadata.dev(),
                ino: metadata.ino(),
                ctime: metadata.ctime(),
                ctime_nsec: metadata.ctime_nsec(),
                mtime: metadata.mtime(),
                mtime_nsec: metadata.mtime_nsec(),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
            }
        }
    }
}

pub fn is_snapshot_spec(spec: &str) -> bool {
    matches!(spec, ":index" | ":worktree" | ":empty") || EMPTY_TREE_IDS.contains(&spec)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffBase {
    Given,
    MergeBase,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitDiff {
    pub base_id: ObjectId,
    pub target_id: ObjectId,
    pub files: Vec<FileOutlineDiff>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDiff {
    pub base_kind: &'static str,
    pub target_kind: &'static str,
    pub base_id: String,
    pub target_id: String,
    pub files: Vec<FileOutlineDiff>,
}

enum SnapshotSide {
    Empty {
        id: String,
    },
    Index {
        entries: BTreeMap<RepoPath, SourceEntry>,
    },
    Worktree {
        entries: BTreeSet<RepoPath>,
    },
    Commit {
        id: ObjectId,
        entries: BTreeMap<RepoPath, SourceEntry>,
    },
}

impl SnapshotSide {
    fn kind(&self) -> &'static str {
        match self {
            Self::Empty { .. } => "empty",
            Self::Index { .. } => "index",
            Self::Worktree { .. } => "worktree",
            Self::Commit { .. } => "commit",
        }
    }

    fn id(&self) -> String {
        match self {
            Self::Empty { id } => id.clone(),
            Self::Index { .. } => ":index".to_owned(),
            Self::Worktree { .. } => ":worktree".to_owned(),
            Self::Commit { id, .. } => id.to_string(),
        }
    }

    fn paths(&self) -> Vec<&RepoPath> {
        match self {
            Self::Empty { .. } => Vec::new(),
            Self::Index { entries } | Self::Commit { entries, .. } => entries.keys().collect(),
            Self::Worktree { entries } => entries.iter().collect(),
        }
    }

    fn entry(&self, path: &RepoPath) -> Option<&SourceEntry> {
        match self {
            Self::Empty { .. } | Self::Worktree { .. } => None,
            Self::Index { entries } | Self::Commit { entries, .. } => entries.get(path),
        }
    }

    fn is_worktree(&self) -> bool {
        matches!(self, Self::Worktree { .. })
    }
}

struct SnapshotSides {
    base: SnapshotSide,
    target: SnapshotSide,
}

impl SnapshotSides {
    fn paths(&self) -> BTreeSet<RepoPath> {
        self.base
            .paths()
            .into_iter()
            .chain(self.target.paths())
            .cloned()
            .collect()
    }
}

// One event in the scope-independent comparison plan. Detection runs over the
// complete snapshots; a scope is applied to these events afterwards, before
// any projection read.
enum PlannedEvent {
    SamePath { path: RepoPath },
    Moved { old: RepoPath, new: RepoPath },
    Added { path: RepoPath },
    Deleted { path: RepoPath },
}

impl PlannedEvent {
    fn target_path(&self) -> &RepoPath {
        match self {
            Self::SamePath { path } | Self::Deleted { path } | Self::Added { path } => path,
            Self::Moved { new, .. } => new,
        }
    }

    fn matches_scope(&self, scope: &PathScope) -> bool {
        match self {
            Self::SamePath { path } | Self::Added { path } | Self::Deleted { path } => {
                scope.matches(path)
            }
            Self::Moved { old, new } => scope.matches(old) || scope.matches(new),
        }
    }
}

struct ComparisonPlan {
    events: Vec<PlannedEvent>,
}

// A path present on one side, before exact or fuzzy pairing. An entry side
// carries its source blob id immediately; a worktree side is hashed only when
// an exact partner with the same language and byte size can exist.
struct Candidate {
    path: RepoPath,
    language: Language,
    entry: Option<SourceEntry>,
    size: Option<u64>,
    identity: Option<ObjectId>,
}

impl Candidate {
    fn is_worktree(&self) -> bool {
        self.entry.is_none()
    }
}

#[derive(Default)]
struct ProfileSlot {
    scalar: Option<PreparedSource>,
    line: Option<PreparedSource>,
}

pub fn project_source(
    path: &RepoPath,
    bytes: &[u8],
    mode: ProjectionMode,
) -> Result<FileOutline, EngineError> {
    let Some(projector) = select_projector(&PROJECTORS, path) else {
        return Err(EngineError::UnsupportedPath { path: path.clone() });
    };
    let path =
        SupportedPath::new(path.clone()).expect("a selected projector implies a supported path");
    let source = decode_source(&path, bytes).map_err(source_projection_failure)?;
    let projection =
        project_items(projector, &path, source, mode).map_err(source_projection_failure)?;
    let superset_mode = mode.superset();
    let superset = if superset_mode == mode {
        projection.clone()
    } else {
        project_items(projector, &path, source, superset_mode).map_err(source_projection_failure)?
    };
    assemble_outline(&path, projection, superset).map_err(source_projection_failure)
}

fn project_items(
    projector: &dyn LanguageProjector,
    path: &SupportedPath,
    source: &str,
    mode: ProjectionMode,
) -> Result<Vec<ProjectedItem>, ProjectionError> {
    Ok(projector
        .project(ProjectionInput { path, source, mode })?
        .items()
        .to_vec())
}

fn source_projection_failure(error: ProjectionError) -> EngineError {
    let context = DiagnosticContext {
        location: Some(Location {
            path: error.supported_path().clone(),
            range: error.range().cloned(),
        }),
        ..DiagnosticContext::default()
    };
    EngineError::Projection {
        context: Box::new(context),
        source: error,
    }
}

pub struct Engine {
    repository: Option<GitRepository>,
    root: PathBuf,
    canonical_root: OnceLock<Option<PathBuf>>,
}

impl Engine {
    pub fn discover(start: &Path) -> Result<Self, EngineError> {
        let repository = GitRepository::discover(start)?;
        // A bare repository has no worktree, so relative paths resolve against
        // its root; `gix` reports the bare root as the Git directory.
        let root = repository
            .work_dir()
            .unwrap_or_else(|| repository.git_dir())
            .to_path_buf();
        Ok(Self {
            repository: Some(repository),
            root,
            canonical_root: OnceLock::new(),
        })
    }

    /// Discover Git when present, otherwise build a filesystem-only engine rooted
    /// at `start`. Only `RepositoryNotFound` becomes a filesystem engine; other
    /// discovery errors remain fatal.
    pub fn discover_or_filesystem(start: &Path) -> Result<Self, EngineError> {
        match Self::discover(start) {
            Ok(engine) => Ok(engine),
            Err(EngineError::Git {
                source: git::GitError::RepositoryNotFound { .. },
                ..
            }) => Ok(Self {
                repository: None,
                root: start.to_path_buf(),
                canonical_root: OnceLock::new(),
            }),
            Err(error) => Err(error),
        }
    }

    pub fn repository(&self) -> Option<&GitRepository> {
        self.repository.as_ref()
    }

    fn require_repository(&self) -> Result<&GitRepository, EngineError> {
        self.repository.as_ref().ok_or_else(|| EngineError::Git {
            revision: None,
            source: git::GitError::RepositoryNotFound {
                start: self.root.clone(),
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "not a git repository",
                )),
            },
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn has_worktree(&self) -> bool {
        self.repository
            .as_ref()
            .map(|repository| repository.work_dir().is_some())
            .unwrap_or(true)
    }

    fn ensure_worktree(&self) -> Result<(), EngineError> {
        if self.has_worktree() {
            Ok(())
        } else {
            Err(EngineError::NoWorktree {
                root: self.root.clone(),
            })
        }
    }

    pub fn first_parent_steps(
        &self,
        base_spec: &str,
        target_spec: &str,
    ) -> Result<Vec<CommitStep>, EngineError> {
        let base = self.require_repository()?.resolve_commit(base_spec)?;
        let target = self.require_repository()?.resolve_commit(target_spec)?;
        Ok(self
            .require_repository()?
            .first_parent_steps(&base, &target)?)
    }

    pub fn load_areas(&self) -> Result<AreaSet, EngineError> {
        Ok(config::Config::load(&self.root)?.areas().clone())
    }

    pub fn show(
        &self,
        revision_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<ProjectedFile>, EngineError> {
        let revision = self.require_repository()?.resolve_commit(revision_spec)?;
        self.ensure_groups(&revision, revision_spec, selection.groups())?;
        let entries = self
            .require_repository()?
            .source_entries(&revision)
            .map_err(|source| EngineError::git(source, Some(revision_spec)))?;

        let projectors: &[&dyn LanguageProjector] = &PROJECTORS;
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for entry in &entries {
            if !selection.scope().matches(&entry.path) {
                continue;
            }
            if let Some(file) =
                self.project_entry(projectors, &mut caches, revision_spec, entry, mode)?
            {
                files.push(file);
            }
        }
        Ok(files)
    }

    pub fn show_outlines(
        &self,
        revision_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<FileOutline>, EngineError> {
        let revision = self.require_repository()?.resolve_commit(revision_spec)?;
        self.ensure_groups(&revision, revision_spec, selection.groups())?;
        let entries = self
            .require_repository()?
            .source_entries(&revision)
            .map_err(|source| EngineError::git(source, Some(revision_spec)))?;

        let projectors: &[&dyn LanguageProjector] = &PROJECTORS;
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for entry in &entries {
            if !selection.scope().matches(&entry.path) {
                continue;
            }
            if let Some(file) =
                self.project_entry_outline(projectors, &mut caches, revision_spec, entry, mode)?
            {
                files.push(file);
            }
        }
        Ok(files)
    }

    pub fn show_source_outlines(
        &self,
        revision_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<SourceOutline>, EngineError> {
        let revision = self.require_repository()?.resolve_commit(revision_spec)?;
        self.ensure_groups(&revision, revision_spec, selection.groups())?;
        let entries = self
            .require_repository()?
            .source_entries(&revision)
            .map_err(|source| EngineError::git(source, Some(revision_spec)))?;

        let projectors: &[&dyn LanguageProjector] = &PROJECTORS;
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for entry in &entries {
            if !selection.scope().matches(&entry.path) {
                continue;
            }
            if let Some((outline, source)) = self.project_entry_source_outline(
                projectors,
                &mut caches,
                revision_spec,
                entry,
                mode,
            )? {
                files.push(SourceOutline { outline, source });
            }
        }
        Ok(files)
    }

    pub fn show_filesystem(
        &self,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<ProjectedFile>, EngineError> {
        self.ensure_worktree()?;
        let paths = self.filesystem_supported_paths()?;
        self.ensure_filesystem_groups(selection.groups())?;
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for path in paths {
            if !selection.scope().matches(&path) {
                continue;
            }
            let Some(bytes) = self.read_worktree_all(&path, &mut caches)? else {
                continue;
            };
            let outline = project_source(&path, &bytes, mode)?;
            files.push(outline.projection);
        }
        Ok(files)
    }

    pub fn show_filesystem_outlines(
        &self,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<FileOutline>, EngineError> {
        self.ensure_worktree()?;
        let paths = self.filesystem_supported_paths()?;
        self.ensure_filesystem_groups(selection.groups())?;
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for path in paths {
            if !selection.scope().matches(&path) {
                continue;
            }
            let Some(bytes) = self.read_worktree_all(&path, &mut caches)? else {
                continue;
            };
            files.push(project_source(&path, &bytes, mode)?);
        }
        Ok(files)
    }

    pub fn show_filesystem_source_outlines(
        &self,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<SourceOutline>, EngineError> {
        self.ensure_worktree()?;
        let paths = self.filesystem_supported_paths()?;
        self.ensure_filesystem_groups(selection.groups())?;
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for path in paths {
            if !selection.scope().matches(&path) {
                continue;
            }
            let Some(bytes) = self.read_worktree_all(&path, &mut caches)? else {
                continue;
            };
            let outline = project_source(&path, &bytes, mode)?;
            let source = std::str::from_utf8(&bytes)
                .map_err(|source| EngineError::WorktreeRead {
                    path: path.clone(),
                    source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
                })?
                .to_owned();
            files.push(SourceOutline { outline, source });
        }
        Ok(files)
    }

    fn filesystem_supported_paths(&self) -> Result<Vec<RepoPath>, EngineError> {
        if let Some(repository) = &self.repository {
            let mut paths: BTreeSet<RepoPath> = repository
                .index_source_entries()
                .map_err(|source| EngineError::git(source, None))?
                .into_iter()
                .map(|entry| entry.path)
                .collect();
            for path in repository
                .untracked_paths()
                .map_err(|source| EngineError::git(source, None))?
            {
                paths.insert(path);
            }
            let mut paths: Vec<RepoPath> = paths.into_iter().collect();
            paths.sort();
            paths.dedup();
            return Ok(paths);
        }

        let mut paths = Vec::new();
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .max_filesize(None)
            .build();

        for result in walker {
            let entry = result.map_err(|source| EngineError::FilesystemTraversal {
                root: self.root.clone(),
                source: source
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("directory walk failed")),
            })?;
            let Some(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let relative = match entry.path().strip_prefix(&self.root) {
                Ok(relative) => relative,
                Err(_) => continue,
            };
            if relative
                .components()
                .any(|component| component.as_os_str() == ".git")
            {
                continue;
            }
            let bytes = path_to_bytes(relative);
            let Ok(repo_path) = RepoPath::new(bytes) else {
                continue;
            };
            if repo_path.language().is_none() {
                continue;
            }
            paths.push(repo_path);
        }

        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn ensure_filesystem_groups(&self, groups: &[SelectionGroup]) -> Result<(), EngineError> {
        let mut missing = Vec::new();
        for group in groups {
            let satisfied = group.paths().iter().any(|path| {
                std::fs::symlink_metadata(self.root.join(worktree_relative(path))).is_ok()
            });
            if !satisfied {
                missing.push(group.clone());
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(EngineError::UnsatisfiedSelection {
                revisions: "filesystem".to_owned(),
                missing,
            })
        }
    }

    pub fn diff(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
        base: DiffBase,
    ) -> Result<Vec<FileDiff>, EngineError> {
        let sides = self.snapshot_sides(base_spec, target_spec, selection, base)?;
        let mut caches = Caches::default();
        let plan = self.comparison_plan(&sides.base, &sides.target, &mut caches);
        let mut diffs = Vec::new();

        for event in plan.events {
            if !event.matches_scope(selection.scope()) {
                continue;
            }
            match event {
                PlannedEvent::SamePath { path } => {
                    let old =
                        self.snapshot_projection(&sides.base, &path, base_spec, mode, &mut caches)?;
                    let new = self.snapshot_projection(
                        &sides.target,
                        &path,
                        target_spec,
                        mode,
                        &mut caches,
                    )?;
                    push_file_diff(&mut diffs, old, new);
                }
                PlannedEvent::Moved { old, new } => {
                    let old_projection =
                        self.snapshot_projection(&sides.base, &old, base_spec, mode, &mut caches)?;
                    let new_projection = self.snapshot_projection(
                        &sides.target,
                        &new,
                        target_spec,
                        mode,
                        &mut caches,
                    )?;
                    push_moved_file_diff(&mut diffs, old_projection, new_projection);
                }
                PlannedEvent::Added { path } => {
                    let new = self.snapshot_projection(
                        &sides.target,
                        &path,
                        target_spec,
                        mode,
                        &mut caches,
                    )?;
                    push_file_diff(&mut diffs, None, new);
                }
                PlannedEvent::Deleted { path } => {
                    let old =
                        self.snapshot_projection(&sides.base, &path, base_spec, mode, &mut caches)?;
                    push_file_diff(&mut diffs, old, None);
                }
            }
        }

        Ok(diffs)
    }

    pub fn diff_outlines(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
        base: DiffBase,
    ) -> Result<CommitDiff, EngineError> {
        let base_revision = self.resolve_base(base_spec, Some(target_spec), base)?;
        let target_revision = self.require_repository()?.resolve_commit(target_spec)?;
        self.ensure_groups_in_diff(
            &base_revision,
            &target_revision,
            base_spec,
            target_spec,
            selection.groups(),
        )?;
        let base_side = self.commit_side(base_spec, base_revision.clone())?;
        let target_side = self.commit_side(target_spec, target_revision.clone())?;

        let mut caches = Caches::default();
        let plan = self.comparison_plan(&base_side, &target_side, &mut caches);
        let mut files = Vec::new();
        for event in plan.events {
            if !event.matches_scope(selection.scope()) {
                continue;
            }
            match event {
                PlannedEvent::SamePath { path } => {
                    let old =
                        self.snapshot_outline(&base_side, &path, base_spec, mode, &mut caches)?;
                    let new =
                        self.snapshot_outline(&target_side, &path, target_spec, mode, &mut caches)?;
                    push_outline_diff(&mut files, old, new);
                }
                PlannedEvent::Moved { old, new } => {
                    let old_outline =
                        self.snapshot_outline(&base_side, &old, base_spec, mode, &mut caches)?;
                    let new_outline =
                        self.snapshot_outline(&target_side, &new, target_spec, mode, &mut caches)?;
                    push_moved_outline_diff(&mut files, old_outline, new_outline);
                }
                PlannedEvent::Added { path } => {
                    let new =
                        self.snapshot_outline(&target_side, &path, target_spec, mode, &mut caches)?;
                    push_outline_diff(&mut files, None, new);
                }
                PlannedEvent::Deleted { path } => {
                    let old =
                        self.snapshot_outline(&base_side, &path, base_spec, mode, &mut caches)?;
                    push_outline_diff(&mut files, old, None);
                }
            }
        }
        Ok(CommitDiff {
            base_id: base_revision.object_id,
            target_id: target_revision.object_id,
            files,
        })
    }

    pub fn diff_snapshot_outlines(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
        base: DiffBase,
    ) -> Result<SnapshotDiff, EngineError> {
        let sides = self.snapshot_sides(base_spec, target_spec, selection, base)?;
        let mut caches = Caches::default();
        let plan = self.comparison_plan(&sides.base, &sides.target, &mut caches);
        let mut files = Vec::new();
        for event in plan.events {
            if !event.matches_scope(selection.scope()) {
                continue;
            }
            match event {
                PlannedEvent::SamePath { path } => {
                    let old =
                        self.snapshot_outline(&sides.base, &path, base_spec, mode, &mut caches)?;
                    let new = self.snapshot_outline(
                        &sides.target,
                        &path,
                        target_spec,
                        mode,
                        &mut caches,
                    )?;
                    push_outline_diff(&mut files, old, new);
                }
                PlannedEvent::Moved { old, new } => {
                    let old_outline =
                        self.snapshot_outline(&sides.base, &old, base_spec, mode, &mut caches)?;
                    let new_outline =
                        self.snapshot_outline(&sides.target, &new, target_spec, mode, &mut caches)?;
                    push_moved_outline_diff(&mut files, old_outline, new_outline);
                }
                PlannedEvent::Added { path } => {
                    let new = self.snapshot_outline(
                        &sides.target,
                        &path,
                        target_spec,
                        mode,
                        &mut caches,
                    )?;
                    push_outline_diff(&mut files, None, new);
                }
                PlannedEvent::Deleted { path } => {
                    let old =
                        self.snapshot_outline(&sides.base, &path, base_spec, mode, &mut caches)?;
                    push_outline_diff(&mut files, old, None);
                }
            }
        }
        Ok(SnapshotDiff {
            base_kind: sides.base.kind(),
            target_kind: sides.target.kind(),
            base_id: sides.base.id(),
            target_id: sides.target.id(),
            files,
        })
    }

    fn snapshot_sides(
        &self,
        base_spec: &str,
        target_spec: &str,
        selection: &Selection,
        base: DiffBase,
    ) -> Result<SnapshotSides, EngineError> {
        let sides = SnapshotSides {
            base: match base {
                DiffBase::Given => self.snapshot_side(base_spec)?,
                DiffBase::MergeBase => self.merge_base_side(base_spec, target_spec)?,
            },
            target: self.snapshot_side(target_spec)?,
        };
        let paths = sides.paths();

        let mut missing = Vec::new();
        for group in selection.groups() {
            if !group
                .paths()
                .iter()
                .any(|prefix| paths.iter().any(|path| path.is_within(prefix)))
            {
                missing.push(group.clone());
            }
        }
        if !missing.is_empty() {
            return Err(EngineError::UnsatisfiedSelection {
                revisions: format!("`{base_spec}` or `{target_spec}`"),
                missing,
            });
        }

        Ok(sides)
    }

    // Build the scope-independent plan over the complete snapshots. Detection
    // runs before scope filtering; only projection reads are scope-limited.
    fn comparison_plan(
        &self,
        base: &SnapshotSide,
        target: &SnapshotSide,
        caches: &mut Caches,
    ) -> ComparisonPlan {
        let mut events = Vec::new();
        let mut old_candidates = Vec::new();
        let mut new_candidates = Vec::new();

        let mut paths: BTreeSet<RepoPath> = base.paths().into_iter().cloned().collect();
        paths.extend(target.paths().into_iter().cloned());

        for path in paths {
            let old_present = self.source_present(base, &path);
            let new_present = self.source_present(target, &path);
            match (old_present, new_present) {
                (true, true) => {
                    if self.same_path_unchanged(base, target, &path) {
                        continue;
                    }
                    events.push(PlannedEvent::SamePath { path });
                }
                (true, false) => old_candidates.push(self.candidate(base, &path)),
                (false, true) => new_candidates.push(self.candidate(target, &path)),
                (false, false) => {}
            }
        }

        self.plan_moves(
            &mut old_candidates,
            &mut new_candidates,
            &mut events,
            caches,
        );
        events.sort_by(|left, right| left.target_path().cmp(right.target_path()));
        ComparisonPlan { events }
    }

    // Snapshot membership is authoritative for scope validation, but a
    // worktree source exists for pairing only when a regular file is actually
    // on disk. Index membership can name a tracked path whose file was moved
    // away without staging; treating that path as present would hide an
    // unstaged rename behind same-path precedence.
    fn source_present(&self, side: &SnapshotSide, path: &RepoPath) -> bool {
        match side {
            SnapshotSide::Empty { .. } => false,
            SnapshotSide::Index { entries } | SnapshotSide::Commit { entries, .. } => {
                entries.contains_key(path)
            }
            SnapshotSide::Worktree { entries } => {
                entries.contains(path) && self.worktree_present(path)
            }
        }
    }

    fn same_path_unchanged(
        &self,
        base: &SnapshotSide,
        target: &SnapshotSide,
        path: &RepoPath,
    ) -> bool {
        if base.is_worktree() && target.is_worktree() {
            return true;
        }
        match (base.entry(path), target.entry(path)) {
            (Some(old), Some(new)) => old.blob_id == new.blob_id,
            _ => false,
        }
    }

    fn candidate(&self, side: &SnapshotSide, path: &RepoPath) -> Candidate {
        let language = path
            .language()
            .expect("snapshot source paths always have a language adapter");
        match side.entry(path) {
            Some(entry) => Candidate {
                path: path.clone(),
                language,
                entry: Some(entry.clone()),
                size: None,
                identity: Some(entry.blob_id.clone()),
            },
            None => Candidate {
                path: path.clone(),
                language,
                entry: None,
                size: self
                    .worktree_regular_file(path)
                    .ok()
                    .flatten()
                    .map(|(_, metadata)| metadata.len()),
                identity: None,
            },
        }
    }

    fn plan_moves(
        &self,
        old: &mut Vec<Candidate>,
        new: &mut Vec<Candidate>,
        events: &mut Vec<PlannedEvent>,
        caches: &mut Caches,
    ) {
        self.prepare_worktree_sizes(old, new);
        self.hash_worktree_identities(old, new, caches);

        let (old_remaining, new_remaining) =
            self.exact_matches(std::mem::take(old), std::mem::take(new), events);
        self.plan_fuzzy(old_remaining, new_remaining, events, caches);
    }

    // Fill sizes for languages that contain a worktree candidate, so the exact
    // pass can skip hashing a worktree file when no opposite candidate can
    // possibly have the same content.
    fn prepare_worktree_sizes(&self, old: &mut [Candidate], new: &mut [Candidate]) {
        let mut languages = BTreeSet::new();
        for candidate in old.iter().chain(new.iter()) {
            if candidate.is_worktree() {
                languages.insert(candidate.language);
            }
        }
        if languages.is_empty() {
            return;
        }
        for candidate in old.iter_mut().chain(new.iter_mut()) {
            if languages.contains(&candidate.language) && candidate.size.is_none() {
                candidate.size = self.candidate_size(candidate);
            }
        }
    }

    fn candidate_size(&self, candidate: &Candidate) -> Option<u64> {
        match &candidate.entry {
            Some(entry) => self
                .require_repository()
                .ok()?
                .blob_size(&entry.blob_id)
                .ok(),
            None => self
                .worktree_regular_file(&candidate.path)
                .ok()
                .flatten()
                .map(|(_, metadata)| metadata.len()),
        }
    }

    fn hash_worktree_identities(
        &self,
        old: &mut [Candidate],
        new: &mut [Candidate],
        caches: &mut Caches,
    ) {
        for candidate in old.iter_mut() {
            if candidate.is_worktree() {
                candidate.identity = self.worktree_identity(candidate, new, caches);
            }
        }
        for candidate in new.iter_mut() {
            if candidate.is_worktree() {
                candidate.identity = self.worktree_identity(candidate, old, caches);
            }
        }
    }

    fn worktree_identity(
        &self,
        candidate: &Candidate,
        opposite: &[Candidate],
        caches: &mut Caches,
    ) -> Option<ObjectId> {
        let size = candidate.size?;
        let can_match = opposite.iter().any(|other| {
            other.language == candidate.language
                && other.size.is_none_or(|other_size| other_size == size)
        });
        if !can_match {
            return None;
        }
        self.hash_worktree_file(&candidate.path, caches)
    }

    fn hash_worktree_file(&self, path: &RepoPath, caches: &mut Caches) -> Option<ObjectId> {
        let (resolved, _) = self.worktree_regular_file(path).ok().flatten()?;
        let mut file = std::fs::File::open(&resolved).ok()?;
        let before = FileFingerprint::from_metadata(&file.metadata().ok()?);
        let identity = if before.len <= WORKTREE_CACHE_MAX_FILE as u64 {
            let mut bytes = Vec::with_capacity(before.len as usize);
            (&mut file)
                .take(before.len + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            let after = FileFingerprint::from_metadata(&file.metadata().ok()?);
            if bytes.len() as u64 != before.len || after != before {
                return None;
            }
            let mut reader = bytes.as_slice();
            let identity = self
                .require_repository()
                .ok()?
                .hash_blob_stream(&mut reader, bytes.len() as u64)
                .ok()?;
            let bytes: Arc<[u8]> = bytes.into();
            cache_worktree_bytes(caches, path, Arc::clone(&bytes));
            identity
        } else {
            let identity = self
                .require_repository()
                .ok()?
                .hash_blob_stream(&mut file, before.len)
                .ok()?;
            let after = FileFingerprint::from_metadata(&file.metadata().ok()?);
            if after != before {
                return None;
            }
            identity
        };
        caches
            .worktree_expected
            .insert(path.clone(), identity.clone());
        Some(identity)
    }

    fn exact_matches(
        &self,
        old: Vec<Candidate>,
        new: Vec<Candidate>,
        events: &mut Vec<PlannedEvent>,
    ) -> (Vec<Candidate>, Vec<Candidate>) {
        let mut old_groups: BTreeMap<(Language, ObjectId), Vec<Candidate>> = BTreeMap::new();
        let mut old_remaining = Vec::new();
        for mut candidate in old {
            match candidate.identity.take() {
                Some(identity) => old_groups
                    .entry((candidate.language, identity))
                    .or_default()
                    .push(candidate),
                None => old_remaining.push(candidate),
            }
        }

        let mut new_groups: BTreeMap<(Language, ObjectId), Vec<Candidate>> = BTreeMap::new();
        let mut new_remaining = Vec::new();
        for mut candidate in new {
            match candidate.identity.take() {
                Some(identity) => new_groups
                    .entry((candidate.language, identity))
                    .or_default()
                    .push(candidate),
                None => new_remaining.push(candidate),
            }
        }

        for (identity, mut olds) in old_groups {
            let Some(mut news) = new_groups.remove(&identity) else {
                old_remaining.extend(olds);
                continue;
            };
            olds.sort_by(|left, right| left.path.cmp(&right.path));
            news.sort_by(|left, right| left.path.cmp(&right.path));
            let paired = olds.len().min(news.len());
            let paired_news = news.drain(..paired).collect::<Vec<_>>();
            for (old, new) in olds.drain(..paired).zip(paired_news) {
                events.push(PlannedEvent::Moved {
                    old: old.path,
                    new: new.path,
                });
            }
            old_remaining.extend(olds);
            new_remaining.extend(news);
        }
        for (_, news) in new_groups {
            new_remaining.extend(news);
        }

        (old_remaining, new_remaining)
    }

    fn plan_fuzzy(
        &self,
        mut old: Vec<Candidate>,
        mut new: Vec<Candidate>,
        events: &mut Vec<PlannedEvent>,
        caches: &mut Caches,
    ) {
        if old.len() > FUZZY_MAX_UNPAIRED_PER_SIDE || new.len() > FUZZY_MAX_UNPAIRED_PER_SIDE {
            push_leftovers(events, old, new);
            return;
        }

        for candidate in old.iter_mut().chain(new.iter_mut()) {
            if candidate.size.is_none() {
                candidate.size = self.candidate_size(candidate);
            }
        }

        let mut eligible = Vec::new();
        for (old_index, old_candidate) in old.iter().enumerate() {
            for (new_index, new_candidate) in new.iter().enumerate() {
                if fuzzy_eligible(old_candidate, new_candidate) {
                    eligible.push((old_index, new_index));
                    if eligible.len() > FUZZY_MAX_ELIGIBLE_PAIRS {
                        push_leftovers(events, old, new);
                        return;
                    }
                }
            }
        }

        let mut old_needs = vec![(false, false); old.len()];
        let mut new_needs = vec![(false, false); new.len()];
        for &(old_index, new_index) in &eligible {
            if pair_uses_scalar_units(&old[old_index], &new[new_index]) {
                old_needs[old_index].0 = true;
                new_needs[new_index].0 = true;
            } else {
                old_needs[old_index].1 = true;
                new_needs[new_index].1 = true;
            }
        }

        // Freeze every candidate that can participate in a fuzzy pair before
        // scoring. Pair scoring never performs IO, and a worktree read happens
        // at most once per candidate regardless of exact-cache occupancy.
        let mut old_sources = vec![None; old.len()];
        let mut new_sources = vec![None; new.len()];
        for (index, candidate) in old.iter().enumerate() {
            if old_needs[index].0 || old_needs[index].1 {
                old_sources[index] = self
                    .fuzzy_snapshot_source(candidate, caches)
                    .filter(|bytes| snapshot_matches_size(candidate, bytes));
            }
        }
        for (index, candidate) in new.iter().enumerate() {
            if new_needs[index].0 || new_needs[index].1 {
                new_sources[index] = self
                    .fuzzy_snapshot_source(candidate, caches)
                    .filter(|bytes| snapshot_matches_size(candidate, bytes));
            }
        }
        debug_assert!(caches.fuzzy_sources_bytes <= FUZZY_SNAPSHOT_MAX_BYTES);

        let mut old_keys = vec![None; old.len()];
        let mut new_keys = vec![None; new.len()];
        for (index, source) in old_sources.iter().enumerate() {
            if let Some(source) = source {
                old_keys[index] = self.fuzzy_source_identity(&old[index], source);
            }
        }
        for (index, source) in new_sources.iter().enumerate() {
            if let Some(source) = source {
                new_keys[index] = self.fuzzy_source_identity(&new[index], source);
            }
        }

        let budget = ProfileBudget::default();
        let mut interner = SimilarityInterner::new();
        let mut old_profiles: Vec<ProfileSlot> =
            (0..old.len()).map(|_| ProfileSlot::default()).collect();
        let mut new_profiles: Vec<ProfileSlot> =
            (0..new.len()).map(|_| ProfileSlot::default()).collect();
        for (index, source) in old_sources.iter().enumerate() {
            if let Some(bytes) = source
                && let Ok(text) = std::str::from_utf8(bytes)
            {
                if old_needs[index].0 {
                    old_profiles[index].scalar =
                        interner.try_prepare(text, UnitMode::Scalar, budget);
                }
                if old_needs[index].1 {
                    old_profiles[index].line = interner.try_prepare(text, UnitMode::Line, budget);
                }
            }
        }
        for (index, source) in new_sources.iter().enumerate() {
            if let Some(bytes) = source
                && let Ok(text) = std::str::from_utf8(bytes)
            {
                if new_needs[index].0 {
                    new_profiles[index].scalar =
                        interner.try_prepare(text, UnitMode::Scalar, budget);
                }
                if new_needs[index].1 {
                    new_profiles[index].line = interner.try_prepare(text, UnitMode::Line, budget);
                }
            }
        }

        let mut score_cache: HashMap<(ObjectId, ObjectId, UnitMode), SimilarityOutcome> =
            HashMap::new();
        let mut scored: Vec<(u8, usize, usize)> = Vec::new();
        for (old_index, new_index) in eligible {
            let scalar = pair_uses_scalar_units(&old[old_index], &new[new_index]);
            let mode = if scalar {
                UnitMode::Scalar
            } else {
                UnitMode::Line
            };
            let (old_profile, new_profile) = if scalar {
                (
                    old_profiles[old_index].scalar.as_ref(),
                    new_profiles[new_index].scalar.as_ref(),
                )
            } else {
                (
                    old_profiles[old_index].line.as_ref(),
                    new_profiles[new_index].line.as_ref(),
                )
            };
            let cache_key = match (old_keys[old_index].clone(), new_keys[new_index].clone()) {
                (Some(old_key), Some(new_key)) => Some((old_key, new_key, mode)),
                _ => None,
            };
            let cached = cache_key
                .as_ref()
                .and_then(|key| score_cache.get(key).copied());
            let outcome = if let Some(outcome) = cached {
                Some(outcome)
            } else {
                let computed = if let (Some(old_profile), Some(new_profile)) =
                    (old_profile, new_profile)
                {
                    Some(source_similarity_at_least_prepared(
                        &interner,
                        old_profile,
                        new_profile,
                        MOVE_SIMILARITY_THRESHOLD,
                    ))
                } else if let (Some(old_source), Some(new_source)) = (
                    old_sources[old_index].as_ref(),
                    new_sources[new_index].as_ref(),
                ) {
                    // The profile budget can leave a pair unprepared. Use the
                    // unchanged byte-slice scorer on the frozen snapshot; the
                    // optimization budget never makes a candidate ineligible.
                    source_similarity_at_least(old_source, new_source, MOVE_SIMILARITY_THRESHOLD)
                } else {
                    None
                };
                if let (Some(key), Some(outcome)) = (cache_key, computed) {
                    score_cache.insert(key, outcome);
                }
                computed
            };
            if let Some(SimilarityOutcome::Score(score)) = outcome {
                scored.push((score, old_index, new_index));
            }
        }

        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| old[left.1].path.cmp(&old[right.1].path))
                .then_with(|| new[left.2].path.cmp(&new[right.2].path))
        });

        let mut used_old = vec![false; old.len()];
        let mut used_new = vec![false; new.len()];
        for (_, old_index, new_index) in scored {
            if used_old[old_index] || used_new[new_index] {
                continue;
            }
            used_old[old_index] = true;
            used_new[new_index] = true;
            events.push(PlannedEvent::Moved {
                old: old[old_index].path.clone(),
                new: new[new_index].path.clone(),
            });
        }

        for (index, candidate) in old.into_iter().enumerate() {
            if !used_old[index] {
                events.push(PlannedEvent::Deleted {
                    path: candidate.path,
                });
            }
        }
        for (index, candidate) in new.into_iter().enumerate() {
            if !used_new[index] {
                events.push(PlannedEvent::Added {
                    path: candidate.path,
                });
            }
        }
    }

    fn fuzzy_source_identity(&self, candidate: &Candidate, source: &[u8]) -> Option<ObjectId> {
        if let Some(entry) = &candidate.entry {
            return Some(entry.blob_id.clone());
        }
        // A worktree candidate may have been hashed before a later capture, so
        // key the score cache by the captured bytes, not by a stale identity.
        let mut reader = source;
        self.require_repository()
            .ok()?
            .hash_blob_stream(&mut reader, source.len() as u64)
            .ok()
    }

    fn fuzzy_snapshot_source(
        &self,
        candidate: &Candidate,
        caches: &mut Caches,
    ) -> Option<Arc<[u8]>> {
        match &candidate.entry {
            Some(entry) => read_blob(
                self.require_repository().unwrap(),
                caches,
                &entry.blob_id,
                "",
            )
            .ok(),
            None => self.snapshot_worktree_source(&candidate.path, caches),
        }
    }

    fn snapshot_worktree_source(&self, path: &RepoPath, caches: &mut Caches) -> Option<Arc<[u8]>> {
        if let Some(bytes) = caches.fuzzy_sources.get(path) {
            return Some(Arc::clone(bytes));
        }
        let bytes = if let Some(bytes) = caches.worktree_bytes.get(path) {
            Arc::clone(bytes)
        } else {
            self.read_worktree_capped(path, caches)?
        };
        if !insert_fuzzy_source(caches, path, Arc::clone(&bytes)) {
            return None;
        }
        Some(bytes)
    }

    fn snapshot_projection(
        &self,
        side: &SnapshotSide,
        path: &RepoPath,
        spec: &str,
        mode: ProjectionMode,
        caches: &mut Caches,
    ) -> Result<Option<ProjectedFile>, EngineError> {
        let entries = match side {
            SnapshotSide::Empty { .. } => return Ok(None),
            SnapshotSide::Index { entries } | SnapshotSide::Commit { entries, .. } => entries,
            SnapshotSide::Worktree { entries } => {
                return if entries.contains(path) {
                    self.project_worktree(path, mode, caches)
                        .map(|outline| outline.map(|outline| outline.projection))
                } else {
                    Ok(None)
                };
            }
        };
        match entries.get(path) {
            Some(entry) => self.project_entry(&PROJECTORS, caches, spec, entry, mode),
            None => Ok(None),
        }
    }

    fn snapshot_side(&self, spec: &str) -> Result<SnapshotSide, EngineError> {
        if spec == ":empty" || EMPTY_TREE_IDS.contains(&spec) {
            return Ok(SnapshotSide::Empty {
                id: spec.to_owned(),
            });
        }
        if spec == ":index" {
            let entries = self
                .require_repository()?
                .index_source_entries()
                .map_err(|e| EngineError::git(e, Some(spec)))?
                .into_iter()
                .map(|entry| (entry.path.clone(), entry))
                .collect();
            return Ok(SnapshotSide::Index { entries });
        }
        if spec == ":worktree" {
            let mut entries: BTreeSet<RepoPath> = self
                .require_repository()?
                .index_source_entries()
                .map_err(|e| EngineError::git(e, Some(spec)))?
                .into_iter()
                .map(|entry| entry.path)
                .collect();
            for path in self
                .require_repository()?
                .untracked_paths()
                .map_err(|e| EngineError::git(e, Some(spec)))?
            {
                entries.insert(path);
            }
            return Ok(SnapshotSide::Worktree { entries });
        }
        let revision = self.require_repository()?.resolve_commit(spec)?;
        self.commit_side(spec, revision)
    }

    fn commit_side(&self, spec: &str, revision: Revision) -> Result<SnapshotSide, EngineError> {
        let entries = self
            .require_repository()?
            .source_entries(&revision)
            .map_err(|e| EngineError::git(e, Some(spec)))?
            .into_iter()
            .map(|entry| (entry.path.clone(), entry))
            .collect();
        Ok(SnapshotSide::Commit {
            id: revision.object_id,
            entries,
        })
    }

    fn merge_base_side(
        &self,
        base_spec: &str,
        target_spec: &str,
    ) -> Result<SnapshotSide, EngineError> {
        let base = self.resolve_base(base_spec, Some(target_spec), DiffBase::MergeBase)?;
        self.commit_side(base_spec, base)
    }

    fn resolve_base(
        &self,
        base_spec: &str,
        target_spec: Option<&str>,
        base: DiffBase,
    ) -> Result<Revision, EngineError> {
        let revision = self.require_repository()?.resolve_commit(base_spec)?;
        match base {
            DiffBase::Given => Ok(revision),
            DiffBase::MergeBase => {
                let target_spec = target_spec.expect("merge-base resolution requires a target");
                let target = self.require_repository()?.resolve_commit(target_spec)?;
                self.require_repository()?
                    .merge_base(&revision, &target)
                    .map_err(|source| EngineError::git(source, Some(base_spec)))
            }
        }
    }

    fn snapshot_outline(
        &self,
        side: &SnapshotSide,
        path: &RepoPath,
        spec: &str,
        mode: ProjectionMode,
        caches: &mut Caches,
    ) -> Result<Option<FileOutline>, EngineError> {
        let entries = match side {
            SnapshotSide::Empty { .. } => return Ok(None),
            SnapshotSide::Index { entries } | SnapshotSide::Commit { entries, .. } => entries,
            SnapshotSide::Worktree { entries } => {
                return if entries.contains(path) {
                    self.project_worktree(path, mode, caches)
                } else {
                    Ok(None)
                };
            }
        };
        match entries.get(path) {
            Some(entry) => self.project_entry_outline(&PROJECTORS, caches, spec, entry, mode),
            None => Ok(None),
        }
    }

    fn project_worktree(
        &self,
        path: &RepoPath,
        mode: ProjectionMode,
        caches: &mut Caches,
    ) -> Result<Option<FileOutline>, EngineError> {
        let Some(bytes) = self.read_worktree_all(path, caches)? else {
            return Ok(None);
        };
        project_source(path, &bytes, mode).map(Some)
    }

    fn worktree_regular_file(
        &self,
        path: &RepoPath,
    ) -> std::io::Result<Option<(PathBuf, std::fs::Metadata)>> {
        let full = self.root.join(worktree_relative(path));
        let metadata = match std::fs::symlink_metadata(&full) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_file() {
            return Ok(None);
        }
        let resolved = full.canonicalize()?;
        let root = self.canonical_root()?;
        if !resolved.starts_with(root) {
            return Ok(None);
        }
        Ok(Some((resolved, metadata)))
    }

    fn canonical_root(&self) -> std::io::Result<&Path> {
        self.canonical_root
            .get_or_init(|| self.root.canonicalize().ok())
            .as_deref()
            .ok_or_else(|| std::io::Error::other("repository root could not be canonicalized"))
    }

    // A path that is missing, a directory, or a symlink is absent for pairing.
    // A regular file that merely cannot be read or is large remains present:
    // discovery is best effort, and a selected projection still reports the
    // existing fatal read/parse failure.
    fn worktree_present(&self, path: &RepoPath) -> bool {
        match self.worktree_regular_file(path) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => true,
        }
    }

    fn read_worktree_all(
        &self,
        path: &RepoPath,
        caches: &mut Caches,
    ) -> Result<Option<Arc<[u8]>>, EngineError> {
        if let Some(bytes) = caches.fuzzy_sources.get(path) {
            return Ok(Some(Arc::clone(bytes)));
        }
        if let Some(bytes) = caches.worktree_bytes.get(path) {
            return Ok(Some(Arc::clone(bytes)));
        }
        let Some((resolved, _)) =
            self.worktree_regular_file(path)
                .map_err(|source| EngineError::WorktreeRead {
                    path: path.clone(),
                    source,
                })?
        else {
            return Ok(None);
        };
        let mut file =
            std::fs::File::open(resolved).map_err(|source| EngineError::WorktreeRead {
                path: path.clone(),
                source,
            })?;
        let before = FileFingerprint::from_metadata(&file.metadata().map_err(|source| {
            EngineError::WorktreeRead {
                path: path.clone(),
                source,
            }
        })?);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| EngineError::WorktreeRead {
                path: path.clone(),
                source,
            })?;
        let after = FileFingerprint::from_metadata(&file.metadata().map_err(|source| {
            EngineError::WorktreeRead {
                path: path.clone(),
                source,
            }
        })?);
        if after != before {
            return Err(worktree_changed(path));
        }
        if let Some(expected) = caches.worktree_expected.get(path) {
            let mut reader = bytes.as_slice();
            let actual = self
                .require_repository()?
                .hash_blob_stream(&mut reader, bytes.len() as u64)
                .map_err(|_| worktree_changed(path))?;
            if actual != *expected {
                return Err(worktree_changed(path));
            }
        }
        let bytes: Arc<[u8]> = bytes.into();
        cache_worktree_bytes(caches, path, Arc::clone(&bytes));
        Ok(Some(bytes))
    }

    fn read_worktree_capped(&self, path: &RepoPath, _caches: &mut Caches) -> Option<Arc<[u8]>> {
        let (resolved, _) = self.worktree_regular_file(path).ok().flatten()?;
        let mut file = std::fs::File::open(resolved).ok()?;
        #[cfg(test)]
        {
            _caches.worktree_snapshot_reads += 1;
        }
        let before = FileFingerprint::from_metadata(&file.metadata().ok()?);
        if before.len > FUZZY_MAX_BYTES {
            return None;
        }
        let mut bytes = Vec::with_capacity(before.len as usize);
        (&mut file)
            .take(FUZZY_MAX_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        let after = FileFingerprint::from_metadata(&file.metadata().ok()?);
        if bytes.len() as u64 != before.len || after != before {
            return None;
        }
        Some(bytes.into())
    }

    fn ensure_groups(
        &self,
        revision: &Revision,
        revision_spec: &str,
        groups: &[SelectionGroup],
    ) -> Result<(), EngineError> {
        let mut missing = Vec::new();
        for group in groups {
            if !self.group_in_revision(revision, revision_spec, group)? {
                missing.push(group.clone());
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(EngineError::UnsatisfiedSelection {
                revisions: format!("`{revision_spec}`"),
                missing,
            })
        }
    }

    fn ensure_groups_in_diff(
        &self,
        base: &Revision,
        target: &Revision,
        base_spec: &str,
        target_spec: &str,
        groups: &[SelectionGroup],
    ) -> Result<(), EngineError> {
        let mut missing = Vec::new();
        for group in groups {
            let present = self.group_in_revision(base, base_spec, group)?
                || self.group_in_revision(target, target_spec, group)?;
            if !present {
                missing.push(group.clone());
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(EngineError::UnsatisfiedSelection {
                revisions: format!("`{base_spec}` or `{target_spec}`"),
                missing,
            })
        }
    }

    fn group_in_revision(
        &self,
        revision: &Revision,
        revision_spec: &str,
        group: &SelectionGroup,
    ) -> Result<bool, EngineError> {
        for path in group.paths() {
            let exists = self
                .require_repository()?
                .path_exists(revision, path)
                .map_err(|source| EngineError::git(source, Some(revision_spec)))?;
            if exists {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn project_entry(
        &self,
        projectors: &[&dyn LanguageProjector],
        caches: &mut Caches,
        revision_spec: &str,
        entry: &SourceEntry,
        mode: ProjectionMode,
    ) -> Result<Option<ProjectedFile>, EngineError> {
        let Some(projector) = select_projector(projectors, &entry.path) else {
            return Ok(None);
        };
        let path = SupportedPath::new(entry.path.clone())
            .expect("a selected projector implies a supported path");
        let items =
            self.project_items_cached(projector, caches, revision_spec, entry, &path, mode)?;
        ProjectedFile::try_new(path, items)
            .map(Some)
            .map_err(|error| self.projection_failure(error, Some(revision_spec)))
    }

    fn project_items_cached(
        &self,
        projector: &dyn LanguageProjector,
        caches: &mut Caches,
        revision_spec: &str,
        entry: &SourceEntry,
        path: &SupportedPath,
        mode: ProjectionMode,
    ) -> Result<Vec<ProjectedItem>, EngineError> {
        if let Some(items) = lookup_projection(caches, &entry.blob_id, path, mode) {
            return Ok(items);
        }

        let source = self.read_source(caches, entry, path, revision_spec)?;
        let items = projector
            .project(ProjectionInput {
                path,
                source: source.as_ref(),
                mode,
            })
            .map_err(|error| self.projection_failure(error, Some(revision_spec)))?
            .items()
            .to_vec();

        caches
            .projections
            .entry(entry.blob_id.clone())
            .or_default()
            .push(CachedProjection {
                path: path.clone(),
                mode,
                items: items.clone(),
            });
        Ok(items)
    }

    fn project_entry_outline(
        &self,
        projectors: &[&dyn LanguageProjector],
        caches: &mut Caches,
        revision_spec: &str,
        entry: &SourceEntry,
        mode: ProjectionMode,
    ) -> Result<Option<FileOutline>, EngineError> {
        Ok(self
            .project_entry_source_outline(projectors, caches, revision_spec, entry, mode)?
            .map(|(outline, _source)| outline))
    }

    fn project_entry_source_outline(
        &self,
        projectors: &[&dyn LanguageProjector],
        caches: &mut Caches,
        revision_spec: &str,
        entry: &SourceEntry,
        mode: ProjectionMode,
    ) -> Result<Option<(FileOutline, String)>, EngineError> {
        let Some(projector) = select_projector(projectors, &entry.path) else {
            return Ok(None);
        };
        let path = SupportedPath::new(entry.path.clone())
            .expect("a selected projector implies a supported path");

        let projection =
            self.project_items_cached(projector, caches, revision_spec, entry, &path, mode)?;
        let superset_mode = mode.superset();
        let superset = if superset_mode == mode {
            projection.clone()
        } else {
            self.project_items_cached(
                projector,
                caches,
                revision_spec,
                entry,
                &path,
                superset_mode,
            )?
        };
        let outline = assemble_outline(&path, projection, superset)
            .map_err(|error| self.projection_failure(error, Some(revision_spec)))?;
        let source = self.read_source(caches, entry, &path, revision_spec)?;
        Ok(Some((outline, source.to_string())))
    }

    fn read_source(
        &self,
        caches: &mut Caches,
        entry: &SourceEntry,
        path: &SupportedPath,
        revision_spec: &str,
    ) -> Result<Arc<str>, EngineError> {
        if let Some(source) = caches.sources.get(&entry.blob_id) {
            return Ok(Arc::clone(source));
        }
        let bytes = read_blob(
            self.require_repository().unwrap(),
            caches,
            &entry.blob_id,
            revision_spec,
        )?;
        let source: Arc<str> = decode_source(path, &bytes)
            .map_err(|error| self.projection_failure(error, Some(revision_spec)))?
            .into();
        caches
            .sources
            .insert(entry.blob_id.clone(), Arc::clone(&source));
        Ok(source)
    }

    fn projection_failure(
        &self,
        error: ProjectionError,
        revision_spec: Option<&str>,
    ) -> EngineError {
        let context = DiagnosticContext {
            repository: Some(self.root.clone()),
            revision: revision_spec.map(str::to_owned),
            location: Some(Location {
                path: error.supported_path().clone(),
                range: error.range().cloned(),
            }),
        };
        EngineError::Projection {
            context: Box::new(context),
            source: error,
        }
    }
}

#[derive(Default)]
struct Caches {
    blobs: HashMap<ObjectId, Arc<[u8]>>,
    sources: HashMap<ObjectId, Arc<str>>,
    projections: HashMap<ObjectId, Vec<CachedProjection>>,
    worktree_bytes: HashMap<RepoPath, Arc<[u8]>>,
    worktree_bytes_total: usize,
    worktree_expected: HashMap<RepoPath, ObjectId>,
    fuzzy_sources: HashMap<RepoPath, Arc<[u8]>>,
    fuzzy_sources_bytes: usize,
    #[cfg(test)]
    worktree_snapshot_reads: usize,
}

fn cache_worktree_bytes(caches: &mut Caches, path: &RepoPath, bytes: Arc<[u8]>) {
    if bytes.len() > WORKTREE_CACHE_MAX_FILE
        || caches.worktree_bytes.contains_key(path)
        || caches.worktree_bytes_total.saturating_add(bytes.len()) > WORKTREE_CACHE_BUDGET
    {
        return;
    }
    caches.worktree_bytes_total += bytes.len();
    caches.worktree_bytes.insert(path.clone(), bytes);
}

// The pre-read size metadata decides caps, ratio, and scalar/line mode. If
// the frozen bytes no longer have that size, the candidate changed between
// metadata and capture: reject it from fuzzy pairing rather than score it with
// a mode that does not describe the captured source.
fn snapshot_matches_size(candidate: &Candidate, bytes: &[u8]) -> bool {
    candidate
        .size
        .is_some_and(|size| size == bytes.len() as u64)
}

fn pair_uses_scalar_units(old: &Candidate, new: &Candidate) -> bool {
    matches!(
        (old.size, new.size),
        (Some(old_size), Some(new_size))
            if old_size <= base::SCALAR_UNIT_MAX_BYTES as u64
                && new_size <= base::SCALAR_UNIT_MAX_BYTES as u64
    )
}

fn worktree_changed(path: &RepoPath) -> EngineError {
    EngineError::WorktreeRead {
        path: path.clone(),
        source: std::io::Error::other("worktree file changed during comparison"),
    }
}

fn insert_fuzzy_source(caches: &mut Caches, path: &RepoPath, bytes: Arc<[u8]>) -> bool {
    if caches.fuzzy_sources.contains_key(path) {
        return true;
    }
    if caches.fuzzy_sources_bytes.saturating_add(bytes.len()) > FUZZY_SNAPSHOT_MAX_BYTES {
        return false;
    }
    caches.fuzzy_sources_bytes += bytes.len();
    caches.fuzzy_sources.insert(path.clone(), bytes);
    true
}

fn fuzzy_eligible(old: &Candidate, new: &Candidate) -> bool {
    if old.language != new.language {
        return false;
    }
    let (Some(old_size), Some(new_size)) = (old.size, new.size) else {
        return false;
    };
    if old_size > FUZZY_MAX_BYTES || new_size > FUZZY_MAX_BYTES {
        return false;
    }
    if old_size == 0 || new_size == 0 {
        return false;
    }
    old_size.min(new_size) * 2 >= old_size.max(new_size)
}

fn push_leftovers(events: &mut Vec<PlannedEvent>, old: Vec<Candidate>, new: Vec<Candidate>) {
    for candidate in old {
        events.push(PlannedEvent::Deleted {
            path: candidate.path,
        });
    }
    for candidate in new {
        events.push(PlannedEvent::Added {
            path: candidate.path,
        });
    }
}

fn push_file_diff(
    output: &mut Vec<FileDiff>,
    old: Option<ProjectedFile>,
    new: Option<ProjectedFile>,
) {
    let old_text = old.as_ref().map_or("", |file| file.canonical_text());
    let new_text = new.as_ref().map_or("", |file| file.canonical_text());
    if old_text == new_text {
        return;
    }
    match (old, new) {
        (Some(old), Some(new)) => output.push(FileDiff::Modified { old, new }),
        (Some(old), None) => output.push(FileDiff::Deleted { old }),
        (None, Some(new)) => output.push(FileDiff::Added { new }),
        (None, None) => {}
    }
}

fn push_moved_file_diff(
    output: &mut Vec<FileDiff>,
    old: Option<ProjectedFile>,
    new: Option<ProjectedFile>,
) {
    match (old, new) {
        (Some(old), Some(new)) => {
            if !(old.canonical_text().is_empty() && new.canonical_text().is_empty()) {
                output.push(FileDiff::Moved { old, new });
            }
        }
        (old, new) => push_file_diff(output, old, new),
    }
}

fn push_outline_diff(
    output: &mut Vec<FileOutlineDiff>,
    old: Option<FileOutline>,
    new: Option<FileOutline>,
) {
    let old_text = old
        .as_ref()
        .map_or("", |file| file.projection.canonical_text());
    let new_text = new
        .as_ref()
        .map_or("", |file| file.projection.canonical_text());
    if old_text == new_text {
        return;
    }
    match (old, new) {
        (Some(old), Some(new)) => output.push(FileOutlineDiff::Modified { old, new }),
        (Some(old), None) => output.push(FileOutlineDiff::Deleted { old }),
        (None, Some(new)) => output.push(FileOutlineDiff::Added { new }),
        (None, None) => {}
    }
}

fn push_moved_outline_diff(
    output: &mut Vec<FileOutlineDiff>,
    old: Option<FileOutline>,
    new: Option<FileOutline>,
) {
    match (old, new) {
        (Some(old), Some(new)) => {
            if !(old.projection.canonical_text().is_empty()
                && new.projection.canonical_text().is_empty())
            {
                output.push(FileOutlineDiff::Moved { old, new });
            }
        }
        (old, new) => push_outline_diff(output, old, new),
    }
}

fn worktree_relative(path: &RepoPath) -> std::ffi::OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(path.as_bytes()).to_os_string()
    }
    #[cfg(not(unix))]
    {
        std::ffi::OsString::from(path.to_string())
    }
}

struct CachedProjection {
    path: SupportedPath,
    mode: ProjectionMode,
    items: Vec<ProjectedItem>,
}

fn lookup_projection(
    caches: &Caches,
    id: &ObjectId,
    path: &SupportedPath,
    mode: ProjectionMode,
) -> Option<Vec<ProjectedItem>> {
    caches.projections.get(id).and_then(|entries| {
        entries
            .iter()
            .find(|entry| entry.path == *path && entry.mode == mode)
            .map(|entry| entry.items.clone())
    })
}

fn read_blob(
    repository: &GitRepository,
    caches: &mut Caches,
    id: &ObjectId,
    revision_spec: &str,
) -> Result<Arc<[u8]>, EngineError> {
    if let Some(bytes) = caches.blobs.get(id) {
        return Ok(Arc::clone(bytes));
    }
    let bytes: Arc<[u8]> = repository
        .read_blob(id)
        .map_err(|source| EngineError::git(source, Some(revision_spec)))?
        .into();
    caches.blobs.insert(id.clone(), Arc::clone(&bytes));
    Ok(bytes)
}

fn path_to_bytes(path: &std::path::Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn test_engine() -> (TempDir, Engine) {
        let directory = TempDir::new().expect("temporary directory");
        let output = Command::new("git")
            .args(["init", "-q"])
            .current_dir(directory.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git init for engine unit test");
        assert!(output.status.success(), "git init failed: {output:?}");
        let engine = Engine::discover(directory.path()).expect("discover test repository");
        (directory, engine)
    }

    fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(path, bytes).expect("write fixture");
    }

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).expect("valid repository path")
    }

    #[test]
    fn fuzzy_snapshot_reads_each_worktree_file_at_most_once() {
        let (directory, engine) = test_engine();
        let mut paths = Vec::new();
        for index in 0..64 {
            let raw = format!("src/file_{index:02}.rs");
            write_file(directory.path(), &raw, b"// frozen fuzzy source\n");
            paths.push(path(&raw));
        }

        let mut caches = Caches::default();
        // Occupying the exact cache for one path must not change the read
        // behavior of the separate fuzzy snapshot.
        cache_worktree_bytes(
            &mut caches,
            &paths[0],
            Arc::from(b"// exact cache bytes\n".as_slice()),
        );

        for candidate in &paths {
            assert!(
                engine
                    .snapshot_worktree_source(candidate, &mut caches)
                    .is_some()
            );
            assert!(
                engine
                    .snapshot_worktree_source(candidate, &mut caches)
                    .is_some()
            );
        }
        assert_eq!(
            caches.worktree_snapshot_reads,
            paths.len() - 1,
            "only the path absent from the exact cache is read, and only once"
        );
        assert_eq!(caches.fuzzy_sources.len(), paths.len());

        for candidate in &paths {
            assert!(
                engine
                    .snapshot_worktree_source(candidate, &mut caches)
                    .is_some()
            );
        }
        assert_eq!(
            caches.worktree_snapshot_reads,
            paths.len() - 1,
            "repeated pair scoring must not read a frozen candidate again"
        );
        assert!(caches.fuzzy_sources_bytes <= FUZZY_SNAPSHOT_MAX_BYTES);
    }

    #[test]
    fn stale_size_metadata_rejects_fuzzy_pairing_but_keeps_captured_projection() {
        let (directory, engine) = test_engine();
        let old_path = path("src/old.rs");
        let new_path = path("src/new.rs");
        let source = vec![b'a'; 5_000];
        write_file(directory.path(), "src/old.rs", &source);
        write_file(directory.path(), "src/new.rs", &source);

        let mut caches = Caches::default();
        let mut events = Vec::new();
        // Metadata says 4 KiB, so the pair would have selected scalar units;
        // the captured files are actually 5 KiB and must not be scored in that
        // mode.
        engine.plan_fuzzy(
            vec![Candidate {
                path: old_path.clone(),
                language: Language::Rust,
                entry: None,
                size: Some(4_096),
                identity: None,
            }],
            vec![Candidate {
                path: new_path.clone(),
                language: Language::Rust,
                entry: None,
                size: Some(4_096),
                identity: None,
            }],
            &mut events,
            &mut caches,
        );

        assert!(
            !events
                .iter()
                .any(|event| matches!(event, PlannedEvent::Moved { .. })),
            "a stale size must not produce a move"
        );
        assert!(matches!(events[0], PlannedEvent::Deleted { .. }));
        assert!(matches!(events[1], PlannedEvent::Added { .. }));

        let projected = engine
            .read_worktree_all(&old_path, &mut caches)
            .expect("captured source is reusable")
            .expect("source exists");
        assert_eq!(
            projected.len(),
            source.len(),
            "the captured bytes remain available to selected projection"
        );
    }

    #[test]
    fn exact_large_worktree_projection_rejects_a_same_size_rewrite() {
        let (directory, engine) = test_engine();
        let candidate = path("src/large.rs");
        let first = vec![b'a'; 300 * 1024];
        write_file(directory.path(), "src/large.rs", &first);

        let mut caches = Caches::default();
        let identity = engine
            .hash_worktree_file(&candidate, &mut caches)
            .expect("hash large worktree file");
        assert_eq!(caches.worktree_expected.get(&candidate), Some(&identity));
        assert!(
            !caches.worktree_bytes.contains_key(&candidate),
            "a file above the fuzzy cap must not be retained as bytes"
        );

        let second = vec![b'b'; first.len()];
        write_file(directory.path(), "src/large.rs", &second);

        let error = engine
            .read_worktree_all(&candidate, &mut caches)
            .expect_err("a changed large source must not be projected as the old identity");
        assert!(
            matches!(error, EngineError::WorktreeRead { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn exact_small_worktree_projection_validates_when_exact_cache_is_gone() {
        let (directory, engine) = test_engine();
        let candidate = path("src/small.rs");
        write_file(directory.path(), "src/small.rs", b"pub fn before() {}\n");

        let mut caches = Caches::default();
        engine
            .hash_worktree_file(&candidate, &mut caches)
            .expect("hash small worktree file");
        assert!(caches.worktree_bytes.contains_key(&candidate));

        // Simulate the exact cache being unavailable at projection time.
        caches.worktree_bytes.clear();
        caches.worktree_bytes_total = 0;

        write_file(directory.path(), "src/small.rs", b"pub fn after!() {}\n");
        let error = engine
            .read_worktree_all(&candidate, &mut caches)
            .expect_err("a same-size rewrite must be rejected");
        assert!(
            matches!(error, EngineError::WorktreeRead { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn fuzzy_snapshot_bytes_are_reused_for_projection() {
        let (directory, engine) = test_engine();
        let candidate = path("src/source.rs");
        let first = b"pub fn captured() {}\n";
        write_file(directory.path(), "src/source.rs", first);

        let mut caches = Caches::default();
        let captured = engine
            .snapshot_worktree_source(&candidate, &mut caches)
            .expect("snapshot worktree source");
        assert_eq!(&*captured, first);

        write_file(directory.path(), "src/source.rs", b"pub fn changed!() {}\n");
        let projected = engine
            .read_worktree_all(&candidate, &mut caches)
            .expect("cached fuzzy source")
            .expect("present worktree source");
        assert_eq!(
            &*projected, first,
            "projection must use the same captured version as scoring"
        );
    }
}
