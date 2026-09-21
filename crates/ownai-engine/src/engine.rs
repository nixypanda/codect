//! The Git-aware projection pipeline shared by every frontend.
//!
//! The engine composes read-only Git snapshot reads with the pure core
//! rendering functions. It owns no long-lived cache: each operation builds and
//! drops its own blob and projection caches, so memory is bounded by one call.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ownai_core::{
    AreaSet, DiagnosticContext, Language, LanguageProjector, ProjectedFile, ProjectedItem,
    ProjectionError, ProjectionInput, ProjectionMode, RepoPath, decode_source, select_projector,
};
use ownai_git::{GitRepository, ObjectId, Revision, SnapshotRepository, SourceEntry};
use ownai_language_elm::ElmProjector;
use ownai_language_rust::RustProjector;

use crate::config;
use crate::error::EngineError;
use crate::selection::{Selection, SelectionGroup};

// ZST projectors are shared as statics so the pipeline never has to own them
// or negotiate a borrow of a longer-lived value.
static ELM_PROJECTOR: ElmProjector = ElmProjector;
static RUST_PROJECTOR: RustProjector = RustProjector;

/// One projected file comparison between two revisions.
///
/// A path present on only one side is an addition or a deletion and carries
/// `None` for the absent projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiff {
    pub path: RepoPath,
    pub old: Option<ProjectedFile>,
    pub new: Option<ProjectedFile>,
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

    /// Reads the current `.ownai.toml` area definitions.
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

        let projectors: [&dyn LanguageProjector; 2] = [&ELM_PROJECTOR, &RUST_PROJECTOR];
        let mut caches = Caches::default();
        let mut files = Vec::new();
        for entry in &entries {
            if !selection.scope().matches(&entry.path) {
                continue;
            }
            if let Some(file) =
                self.project_entry(&projectors, &mut caches, revision_spec, entry, mode)?
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

        let projectors: [&dyn LanguageProjector; 2] = [&ELM_PROJECTOR, &RUST_PROJECTOR];
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
                    self.project_entry(&projectors, &mut caches, base_spec, entry, mode)?
                }
                None => None,
            };
            let new = match new_entry {
                Some(entry) => {
                    self.project_entry(&projectors, &mut caches, target_spec, entry, mode)?
                }
                None => None,
            };

            let old_text = old.as_ref().map_or("", |file| file.canonical_text());
            let new_text = new.as_ref().map_or("", |file| file.canonical_text());
            if old_text == new_text {
                continue;
            }
            diffs.push(FileDiff {
                path: path.clone(),
                old,
                new,
            });
        }

        Ok(diffs)
    }

    /// Fails when a selected group names nothing in `revision`, before any blob
    /// is read, so a mistyped path or area cannot masquerade as an empty
    /// projection. An area is satisfied by any one of its paths existing.
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

    /// Fails when a selected group is absent from both revisions, before any
    /// blob is read. A path deleted by the target still belongs to the diff, so
    /// a group counts as present wherever either side names it.
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

    /// Reads, decodes, and projects one entry, reusing the caches.
    ///
    /// Returns `None` for an unsupported path, which is an exclusion rather
    /// than a failure.
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
        let language = projector.language();

        if let Some(items) = lookup_projection(caches, &entry.blob_id, language, mode) {
            // The cache key intentionally excludes the path. Reusing the items
            // for another path is safe because stable keys and spans are never
            // rendered; only `canonical_text` reaches a document.
            return Ok(Some(ProjectedFile::new(
                entry.path.clone(),
                language,
                items,
            )));
        }

        let bytes = read_blob(&self.repository, caches, &entry.blob_id, revision_spec)?;
        let source = decode_source(&entry.path, language, &bytes)
            .map_err(|error| self.projection_failure(error, revision_spec))?;
        let projected = projector
            .project(ProjectionInput {
                path: &entry.path,
                source,
                mode,
            })
            .map_err(|error| self.projection_failure(error, revision_spec))?;

        let items = projected.items().to_vec();
        caches
            .projections
            .entry(entry.blob_id.clone())
            .or_default()
            .push(CachedProjection {
                language,
                mode,
                items: items.clone(),
            });
        Ok(Some(ProjectedFile::new(
            entry.path.clone(),
            language,
            items,
        )))
    }

    fn projection_failure(&self, error: ProjectionError, revision_spec: &str) -> EngineError {
        let context = DiagnosticContext {
            repository: Some(self.repository.git_dir().to_path_buf()),
            revision: Some(revision_spec.to_owned()),
            path: Some(error.path().clone()),
            language: Some(error.language()),
            range: error.range().cloned(),
        };
        EngineError::Projection {
            context: Box::new(context),
            source: error,
        }
    }
}

/// Per-operation caches; never persisted.
#[derive(Default)]
struct Caches {
    blobs: HashMap<ObjectId, Arc<[u8]>>,
    projections: HashMap<ObjectId, Vec<CachedProjection>>,
}

struct CachedProjection {
    language: Language,
    mode: ProjectionMode,
    items: Vec<ProjectedItem>,
}

fn lookup_projection(
    caches: &Caches,
    id: &ObjectId,
    language: Language,
    mode: ProjectionMode,
) -> Option<Vec<ProjectedItem>> {
    caches.projections.get(id).and_then(|entries| {
        entries
            .iter()
            .find(|entry| entry.language == language && entry.mode == mode)
            .map(|entry| entry.items.clone())
    })
}

/// Reads each blob at most once per operation, sharing it via `Arc` so callers
/// can borrow the bytes while the cache stays mutable for later reads.
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
