//! The Git-aware projection pipeline shared by every frontend.
//!
//! The engine composes read-only Git snapshot reads with the pure core
//! rendering functions. It owns no long-lived cache: each operation builds and
//! drops its own blob and projection caches, so memory is bounded by one call.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base::{
    AreaSet, DiagnosticContext, FileDiff, FileOutline, FileOutlineDiff, LanguageProjector,
    Location, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput, ProjectionMode,
    RepoPath, Selection, SelectionGroup, SupportedPath, assemble_outline, decode_source,
    select_projector,
};
use git::{CommitStep, GitRepository, ObjectId, Revision, SnapshotRepository, SourceEntry};
use lang_elm::ElmProjector;
use lang_haskell::HaskellProjector;
use lang_python::PythonProjector;
use lang_rust::RustProjector;

use crate::config;
use crate::error::EngineError;

// ZST projectors are shared as statics so the pipeline never has to own them
// or negotiate a borrow of a longer-lived value.
static ELM_PROJECTOR: ElmProjector = ElmProjector;
static HASKELL_PROJECTOR: HaskellProjector = HaskellProjector;
static PYTHON_PROJECTOR: PythonProjector = PythonProjector;
static RUST_PROJECTOR: RustProjector = RustProjector;

// The language adapters, in a fixed order so path selection is deterministic.
//
// Every new language is added here once; the pipeline stays language-agnostic.
const PROJECTORS: [&dyn LanguageProjector; 4] = [
    &ELM_PROJECTOR,
    &HASKELL_PROJECTOR,
    &PYTHON_PROJECTOR,
    &RUST_PROJECTOR,
];

/// A commit-to-commit focused comparison with immutable snapshot identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitDiff {
    pub base_id: ObjectId,
    pub target_id: ObjectId,
    pub files: Vec<FileOutlineDiff>,
}

/// A focused comparison whose sides may come from commits, the index, the
/// working tree, or an empty tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDiff {
    pub base_kind: &'static str,
    pub target_kind: &'static str,
    pub base_id: String,
    pub target_id: String,
    pub files: Vec<FileOutlineDiff>,
}

#[derive(Clone, Debug)]
enum SnapshotEntry {
    Blob(SourceEntry),
    Worktree,
}

type SnapshotEntries = (&'static str, String, Vec<(RepoPath, SnapshotEntry)>);

/// Projects explicit source bytes for `path`, returning the requested-mode
/// projection plus a mode-independent outline.
///
/// This is the editor-facing entry point: it never touches Git and needs no
/// repository, so a caller can project a buffer's bytes directly. An unsupported
/// path is a typed [`EngineError::UnsupportedPath`] because the caller supplied
/// the path as the projection context rather than as a scope.
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
    let superset = if mode == ProjectionMode::Signatures {
        projection.clone()
    } else {
        project_items(projector, &path, source, ProjectionMode::Signatures)
            .map_err(source_projection_failure)?
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

/// A discovered repository plus the pipeline over it.
pub struct Engine {
    repository: GitRepository,
    root: PathBuf,
}

impl Engine {
    /// Discovers the repository that contains `start`.
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

    /// The repository root relative paths resolve against.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves both endpoints once and lists the target's first-parent steps
    /// through the base, newest first. Projection is deferred until a caller
    /// requests a selected step with [`Self::diff_outlines`].
    pub fn first_parent_steps(
        &self,
        base_spec: &str,
        target_spec: &str,
    ) -> Result<Vec<CommitStep>, EngineError> {
        let base = self.repository.resolve_commit(base_spec)?;
        let target = self.repository.resolve_commit(target_spec)?;
        Ok(self.repository.first_parent_steps(&base, &target)?)
    }

    /// Reads the current `.codect.toml` area definitions.
    ///
    /// The engine reloads them on every call; callers decide how long to retain
    /// the owned snapshot.
    pub fn load_areas(&self) -> Result<AreaSet, EngineError> {
        Ok(config::Config::load(&self.root)?.areas().clone())
    }

    /// Projects every selected file of `revision`.
    ///
    /// Supported selected files are returned in raw path-byte order, including
    /// files whose canonical projection is empty. A selection that names
    /// nothing is rejected before any blob is read.
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

    /// Projects every selected file of `revision`, pairing the requested-mode
    /// projection with a complete, mode-independent outline.
    ///
    /// This is the JSON `show` path. It reads exactly the same committed blobs
    /// as [`Engine::show`], in the same raw path-byte order, so the outline and
    /// `stable_key` values match the text document for the same revision.
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

    /// Compares the projections of `base_spec` and `target_spec`.
    ///
    /// Only paths whose canonical old and new projections differ are returned,
    /// in raw path-byte order. Implementation-only changes therefore do not
    /// appear at all.
    pub fn diff(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<Vec<FileDiff>, EngineError> {
        let base = self.repository.resolve_commit(base_spec)?;
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

        // A `BTreeSet` merges the two sides in raw path byte order, which is the
        // order the result uses.
        let mut paths: BTreeSet<&RepoPath> = base_map.keys().copied().collect();
        paths.extend(target_map.keys().copied());

        let projectors: &[&dyn LanguageProjector] = &PROJECTORS;
        let mut caches = Caches::default();
        let mut diffs = Vec::new();

        for path in paths {
            if !selection.scope().matches(path) {
                continue;
            }
            let old_entry = base_map.get(path).copied();
            let new_entry = target_map.get(path).copied();

            // Skip identical blobs before reading or projecting either side.
            // This is the main performance rule and is also what makes an
            // unchanged file invisible.
            if let (Some(old), Some(new)) = (old_entry, new_entry)
                && old.blob_id == new.blob_id
            {
                continue;
            }

            let old = match old_entry {
                Some(entry) => {
                    self.project_entry(projectors, &mut caches, base_spec, entry, mode)?
                }
                None => None,
            };
            let new = match new_entry {
                Some(entry) => {
                    self.project_entry(projectors, &mut caches, target_spec, entry, mode)?
                }
                None => None,
            };

            let old_text = old.as_ref().map_or("", |file| file.canonical_text());
            let new_text = new.as_ref().map_or("", |file| file.canonical_text());
            if old_text == new_text {
                continue;
            }
            diffs.push(match (old, new) {
                (None, Some(new)) => FileDiff::Added { new },
                (Some(old), None) => FileDiff::Deleted { old },
                (Some(old), Some(new)) => FileDiff::Modified { old, new },
                (None, None) => continue, // unreachable after the entry checks
            });
        }

        Ok(diffs)
    }

    /// Compares committed projections and returns both panes with their outlines.
    /// Equal projections, unsupported files, and empty added/deleted projections
    /// are omitted, matching the text diff's focused file-list rule.
    pub fn diff_outlines(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<CommitDiff, EngineError> {
        let base = self.repository.resolve_commit(base_spec)?;
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
            files.push(match (old, new) {
                (None, Some(new)) => FileOutlineDiff::Added { new },
                (Some(old), None) => FileOutlineDiff::Deleted { old },
                (Some(old), Some(new)) => FileOutlineDiff::Modified { old, new },
                (None, None) => continue, // unreachable after the entry checks
            });
        }
        Ok(CommitDiff {
            base_id: base.object_id,
            target_id: target.object_id,
            files,
        })
    }

    /// Compares any two read-only snapshot sources for the JSON Diffview
    /// contract. `:index`, `:worktree`, and `:empty` are reserved specifiers;
    /// the canonical Git empty-tree object id is also accepted as `:empty`.
    pub fn diff_snapshot_outlines(
        &self,
        base_spec: &str,
        target_spec: &str,
        mode: ProjectionMode,
        selection: &Selection,
    ) -> Result<SnapshotDiff, EngineError> {
        let (base_kind, base_id, base_entries) = self.snapshot_entries(base_spec)?;
        let (target_kind, target_id, target_entries) = self.snapshot_entries(target_spec)?;
        let base_map: BTreeMap<RepoPath, SnapshotEntry> = base_entries.into_iter().collect();
        let target_map: BTreeMap<RepoPath, SnapshotEntry> = target_entries.into_iter().collect();
        let mut paths: BTreeSet<RepoPath> = base_map.keys().cloned().collect();
        paths.extend(target_map.keys().cloned());

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

        let mut caches = Caches::default();
        let mut files = Vec::new();
        for path in paths {
            if !selection.scope().matches(&path) {
                continue;
            }
            let old = self.snapshot_outline(
                base_map.get(&path),
                &path,
                base_kind,
                base_spec,
                mode,
                &mut caches,
            )?;
            let new = self.snapshot_outline(
                target_map.get(&path),
                &path,
                target_kind,
                target_spec,
                mode,
                &mut caches,
            )?;
            if old.as_ref().map_or("", |f| f.projection.canonical_text())
                == new.as_ref().map_or("", |f| f.projection.canonical_text())
            {
                continue;
            }
            files.push(match (old, new) {
                (None, Some(new)) => FileOutlineDiff::Added { new },
                (Some(old), None) => FileOutlineDiff::Deleted { old },
                (Some(old), Some(new)) => FileOutlineDiff::Modified { old, new },
                (None, None) => continue, // unreachable after the entry checks
            });
        }
        Ok(SnapshotDiff {
            base_kind,
            target_kind,
            base_id,
            target_id,
            files,
        })
    }

    fn snapshot_entries(&self, spec: &str) -> Result<SnapshotEntries, EngineError> {
        if spec == ":empty"
            || matches!(
                spec,
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                    | "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"
            )
        {
            return Ok(("empty", spec.to_owned(), Vec::new()));
        }
        if spec == ":index" || spec == ":worktree" {
            let entries = self
                .repository
                .index_source_entries()
                .map_err(|e| EngineError::git(e, Some(spec)))?;
            let entries = entries
                .into_iter()
                .map(|entry| {
                    let path = entry.path.clone();
                    let value = if spec == ":index" {
                        SnapshotEntry::Blob(entry)
                    } else {
                        SnapshotEntry::Worktree
                    };
                    (path, value)
                })
                .collect();
            return Ok((
                if spec == ":index" {
                    "index"
                } else {
                    "worktree"
                },
                spec.to_owned(),
                entries,
            ));
        }
        let revision = self.repository.resolve_commit(spec)?;
        let id = revision.object_id.to_string();
        let entries = self
            .repository
            .source_entries(&revision)
            .map_err(|e| EngineError::git(e, Some(spec)))?
            .into_iter()
            .map(|entry| (entry.path.clone(), SnapshotEntry::Blob(entry)))
            .collect();
        Ok(("commit", id, entries))
    }

    fn snapshot_outline(
        &self,
        entry: Option<&SnapshotEntry>,
        path: &RepoPath,
        kind: &str,
        spec: &str,
        mode: ProjectionMode,
        caches: &mut Caches,
    ) -> Result<Option<FileOutline>, EngineError> {
        match entry {
            Some(SnapshotEntry::Blob(entry)) => {
                self.project_entry_outline(&PROJECTORS, caches, spec, entry, mode)
            }
            Some(SnapshotEntry::Worktree) => self.project_worktree(path, mode),
            None if kind == "worktree" => self.project_worktree(path, mode),
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

    // Fails when a selected group names nothing in `revision`, before any blob
    // is read, so a mistyped path or area cannot masquerade as an empty
    // projection. An area is satisfied by any one of its paths existing.
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

    // Fails when a selected group is absent from both revisions, before any
    // blob is read. A path deleted by the target still belongs to the diff, so
    // a group counts as present wherever either side names it.
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
            // Short-circuiting means the target is only consulted when the base
            // does not already satisfy the group.
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

    // Reads, decodes, and projects one entry, reusing the caches.
    //
    // Returns `None` for an unsupported path, which is an exclusion rather
    // than a failure.
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

    // Projects one entry's items in `mode`, reading the blob once and reusing
    // the `(blob id, path, mode)` cache.
    //
    // The cache key includes the path because `stable_key` is path-namespaced
    // for several languages (Python, Haskell, Elm, and Rust `mod`): those keys
    // embed the repository path, so items projected for one path are not valid
    // for a different path that happens to share the same blob. Blob bytes are
    // still cached on the blob id alone, so an identical blob is read once. The
    // path carries its derived language, so the key also distinguishes
    // languages without a separate field.
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

        let bytes = read_blob(&self.repository, caches, &entry.blob_id, revision_spec)?;
        let source = decode_source(path, &bytes)
            .map_err(|error| self.projection_failure(error, Some(revision_spec)))?;
        let items = projector
            .project(ProjectionInput { path, source, mode })
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

    // Reads, decodes, and projects one entry in both the requested mode and the
    // Signatures superset, assembling the file's outline.
    //
    // Both projections go through the shared `(blob id, path, mode)` cache, so
    // a blob is read once and each mode is computed at most once per path and
    // operation.
    //
    // Returns `None` for an unsupported path, which is an exclusion rather
    // than a failure.
    fn project_entry_outline(
        &self,
        projectors: &[&dyn LanguageProjector],
        caches: &mut Caches,
        revision_spec: &str,
        entry: &SourceEntry,
        mode: ProjectionMode,
    ) -> Result<Option<FileOutline>, EngineError> {
        let Some(projector) = select_projector(projectors, &entry.path) else {
            return Ok(None);
        };
        let path = SupportedPath::new(entry.path.clone())
            .expect("a selected projector implies a supported path");

        let projection =
            self.project_items_cached(projector, caches, revision_spec, entry, &path, mode)?;
        let superset = if mode == ProjectionMode::Signatures {
            projection.clone()
        } else {
            self.project_items_cached(
                projector,
                caches,
                revision_spec,
                entry,
                &path,
                ProjectionMode::Signatures,
            )?
        };
        assemble_outline(&path, projection, superset)
            .map(Some)
            .map_err(|error| self.projection_failure(error, Some(revision_spec)))
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

// Per-operation caches; never persisted.
#[derive(Default)]
struct Caches {
    blobs: HashMap<ObjectId, Arc<[u8]>>,
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

// Reads each blob at most once per operation, sharing it via `Arc` so callers
// can borrow the bytes while the cache stays mutable for later reads.
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
