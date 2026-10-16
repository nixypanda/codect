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
//! projections differ, so an implementation-only change produces nothing.
//! [`project_source`] is the editor-facing entry point: it never touches Git,
//! and an unsupported path is a typed [`EngineError::UnsupportedPath`] rather
//! than a silent empty result.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base::{
    AreaSet, DiagnosticContext, FileDiff, FileOutline, FileOutlineDiff, LanguageProjector,
    Location, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput, ProjectionMode,
    RepoPath, Selection, SelectionGroup, SourceOutline, SupportedPath, assemble_outline,
    decode_source, select_projector,
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
    repository: GitRepository,
    root: PathBuf,
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
        Ok(Self { repository, root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn first_parent_steps(
        &self,
        base_spec: &str,
        target_spec: &str,
    ) -> Result<Vec<CommitStep>, EngineError> {
        let base = self.repository.resolve_commit(base_spec)?;
        let target = self.repository.resolve_commit(target_spec)?;
        Ok(self.repository.first_parent_steps(&base, &target)?)
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
        let revision = self.repository.resolve_commit(revision_spec)?;
        self.ensure_groups(&revision, revision_spec, selection.groups())?;
        let entries = self
            .repository
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
        let revision = self.repository.resolve_commit(revision_spec)?;
        self.ensure_groups(&revision, revision_spec, selection.groups())?;
        let entries = self
            .repository
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
        let revision = self.repository.resolve_commit(revision_spec)?;
        self.ensure_groups(&revision, revision_spec, selection.groups())?;
        let entries = self
            .repository
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

    pub fn diff(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
        base: DiffBase,
    ) -> Result<Vec<FileDiff>, EngineError> {
        let sides = self.snapshot_sides(base_spec, target_spec, selection, base)?;
        let paths = sides.paths();
        let mut caches = Caches::default();
        let mut diffs = Vec::new();

        for path in &paths {
            if !selection.scope().matches(path) {
                continue;
            }

            if let (
                SnapshotSide::Index { entries: old } | SnapshotSide::Commit { entries: old, .. },
                SnapshotSide::Index { entries: new } | SnapshotSide::Commit { entries: new, .. },
            ) = (&sides.base, &sides.target)
                && let (Some(old), Some(new)) = (old.get(path), new.get(path))
                && old.blob_id == new.blob_id
            {
                continue;
            }

            let old = self.snapshot_projection(&sides.base, path, base_spec, mode, &mut caches)?;
            let new =
                self.snapshot_projection(&sides.target, path, target_spec, mode, &mut caches)?;

            let old_text = old.as_ref().map_or("", |file| file.canonical_text());
            let new_text = new.as_ref().map_or("", |file| file.canonical_text());
            if old_text == new_text {
                continue;
            }
            if let Some(new) = new {
                if let Some(old) = old {
                    diffs.push(FileDiff::Modified { old, new });
                } else {
                    diffs.push(FileDiff::Added { new });
                }
            } else if let Some(old) = old {
                diffs.push(FileDiff::Deleted { old });
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
        let base = self.resolve_base(base_spec, Some(target_spec), base)?;
        let target = self.repository.resolve_commit(target_spec)?;
        self.ensure_groups_in_diff(&base, &target, base_spec, target_spec, selection.groups())?;
        let base_entries = self
            .repository
            .source_entries(&base)
            .map_err(|source| EngineError::git(source, Some(base_spec)))?;
        let target_entries = self
            .repository
            .source_entries(&target)
            .map_err(|source| EngineError::git(source, Some(target_spec)))?;
        let base_map: BTreeMap<&RepoPath, &SourceEntry> = base_entries
            .iter()
            .map(|entry| (&entry.path, entry))
            .collect();
        let target_map: BTreeMap<&RepoPath, &SourceEntry> = target_entries
            .iter()
            .map(|entry| (&entry.path, entry))
            .collect();
        let mut paths: BTreeSet<&RepoPath> = base_map.keys().copied().collect();
        paths.extend(target_map.keys().copied());

        let mut caches = Caches::default();
        let mut files = Vec::new();
        for path in paths {
            if !selection.scope().matches(path) {
                continue;
            }
            let old_entry = base_map.get(path).copied();
            let new_entry = target_map.get(path).copied();
            if let (Some(old), Some(new)) = (old_entry, new_entry)
                && old.blob_id == new.blob_id
            {
                continue;
            }
            let old = match old_entry {
                Some(entry) => {
                    self.project_entry_outline(&PROJECTORS, &mut caches, base_spec, entry, mode)?
                }
                None => None,
            };
            let new = match new_entry {
                Some(entry) => {
                    self.project_entry_outline(&PROJECTORS, &mut caches, target_spec, entry, mode)?
                }
                None => None,
            };
            if old
                .as_ref()
                .map_or("", |file| file.projection.canonical_text())
                == new
                    .as_ref()
                    .map_or("", |file| file.projection.canonical_text())
            {
                continue;
            }
            if let Some(new) = new {
                if let Some(old) = old {
                    files.push(FileOutlineDiff::Modified { old, new });
                } else {
                    files.push(FileOutlineDiff::Added { new });
                }
            } else if let Some(old) = old {
                files.push(FileOutlineDiff::Deleted { old });
            }
        }
        Ok(CommitDiff {
            base_id: base.object_id,
            target_id: target.object_id,
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
        let paths = sides.paths();
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for path in &paths {
            if !selection.scope().matches(path) {
                continue;
            }
            let old = self.snapshot_outline(&sides.base, path, base_spec, mode, &mut caches)?;
            let new = self.snapshot_outline(&sides.target, path, target_spec, mode, &mut caches)?;
            if old.as_ref().map_or("", |f| f.projection.canonical_text())
                == new.as_ref().map_or("", |f| f.projection.canonical_text())
            {
                continue;
            }
            if let Some(new) = new {
                if let Some(old) = old {
                    files.push(FileOutlineDiff::Modified { old, new });
                } else {
                    files.push(FileOutlineDiff::Added { new });
                }
            } else if let Some(old) = old {
                files.push(FileOutlineDiff::Deleted { old });
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
                    self.project_worktree(path, mode)
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
                .repository
                .index_source_entries()
                .map_err(|e| EngineError::git(e, Some(spec)))?
                .into_iter()
                .map(|entry| (entry.path.clone(), entry))
                .collect();
            return Ok(SnapshotSide::Index { entries });
        }
        if spec == ":worktree" {
            let mut entries: BTreeSet<RepoPath> = self
                .repository
                .index_source_entries()
                .map_err(|e| EngineError::git(e, Some(spec)))?
                .into_iter()
                .map(|entry| entry.path)
                .collect();
            for path in self
                .repository
                .untracked_paths()
                .map_err(|e| EngineError::git(e, Some(spec)))?
            {
                entries.insert(path);
            }
            return Ok(SnapshotSide::Worktree { entries });
        }
        let revision = self.repository.resolve_commit(spec)?;
        self.commit_side(spec, revision)
    }

    fn commit_side(&self, spec: &str, revision: Revision) -> Result<SnapshotSide, EngineError> {
        let entries = self
            .repository
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
        let revision = self.repository.resolve_commit(base_spec)?;
        match base {
            DiffBase::Given => Ok(revision),
            DiffBase::MergeBase => {
                let target_spec = target_spec.expect("merge-base resolution requires a target");
                let target = self.repository.resolve_commit(target_spec)?;
                self.repository
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
                    self.project_worktree(path, mode)
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
    ) -> Result<Option<FileOutline>, EngineError> {
        #[cfg(unix)]
        let relative = {
            use std::os::unix::ffi::OsStrExt;
            std::ffi::OsStr::from_bytes(path.as_bytes()).to_os_string()
        };
        #[cfg(not(unix))]
        let relative = std::ffi::OsString::from(path.to_string());
        let full = self.root.join(relative);
        let metadata = match std::fs::symlink_metadata(&full) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(EngineError::WorktreeRead {
                    path: path.clone(),
                    source,
                });
            }
        };
        if !metadata.file_type().is_file() {
            return Ok(None);
        }
        let resolved = full
            .canonicalize()
            .map_err(|source| EngineError::WorktreeRead {
                path: path.clone(),
                source,
            })?;
        let root = self
            .root
            .canonicalize()
            .map_err(|source| EngineError::WorktreeRead {
                path: path.clone(),
                source,
            })?;
        if !resolved.starts_with(root) {
            return Ok(None);
        }
        let bytes = std::fs::read(resolved).map_err(|source| EngineError::WorktreeRead {
            path: path.clone(),
            source,
        })?;
        project_source(path, &bytes, mode).map(Some)
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
                .repository
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
        let bytes = read_blob(&self.repository, caches, &entry.blob_id, revision_spec)?;
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
            repository: Some(self.repository.git_dir().to_path_buf()),
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
