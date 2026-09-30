//! Repository discovery and read-only opening.
//!
//! OwnAI-owned value types and the [`SnapshotRepository`] trait mirror
//! TECHNICAL_DESIGN.md section 8; no `gix` type appears in a public signature.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::GitError;

// The object cache budget for one opened repository (TECHNICAL_DESIGN.md
// section 8.4).
//
// The value is deliberately conservative for the MVP and is kept inside this
// crate; there are no tuning flags. It bounds the fully decoded object cache
// used while reading trees and blobs repeatedly.
const OBJECT_CACHE_SIZE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HashKind {
    Sha1,
    Sha256,
}

impl HashKind {
    // `gix` types must stay inside this crate. With the approved feature set
    // (`sha1` and `sha256`) only those two kinds can occur.
    pub(crate) fn from_gix(kind: gix::hash::Kind) -> Self {
        match kind {
            gix::hash::Kind::Sha1 => Self::Sha1,
            gix::hash::Kind::Sha256 => Self::Sha256,
            _ => Self::Sha256,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId {
    pub kind: HashKind,
    pub bytes: Vec<u8>,
}

impl ObjectId {
    pub(crate) fn from_gix(id: &gix::hash::ObjectId) -> Self {
        Self {
            kind: HashKind::from_gix(id.kind()),
            bytes: id.as_slice().to_vec(),
        }
    }

    // The length and declared [`HashKind`] must agree; a mismatch is
    // [`GitError::InvalidObjectId`] rather than a panic.
    pub(crate) fn to_gix(&self, repository: &Path) -> Result<gix::hash::ObjectId, GitError> {
        let id = gix::hash::ObjectId::try_from(self.bytes.as_slice()).map_err(|_| {
            GitError::InvalidObjectId {
                repository: repository.to_path_buf(),
                object_id: self.clone(),
            }
        })?;

        if HashKind::from_gix(id.kind()) != self.kind {
            return Err(GitError::InvalidObjectId {
                repository: repository.to_path_buf(),
                object_id: self.clone(),
            });
        }

        Ok(id)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.bytes {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Revision {
    pub object_id: ObjectId,
}

/// One step on the target's first-parent chain, compared with its first parent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitStep {
    pub parent_id: ObjectId,
    pub commit_id: ObjectId,
    pub subject: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEntry {
    pub path: base::RepoPath,
    pub blob_id: ObjectId,
    pub executable: bool,
}

/// Read-only access to committed snapshots of a repository.
pub trait SnapshotRepository {
    /// Resolves `spec` to exactly one commit, peeling tags and other
    /// commit-ish objects.
    fn resolve_commit(&self, spec: &str) -> Result<Revision, GitError>;

    /// Lists steps after `base` through `target`, newest first. Both inputs
    /// must already be resolved commits, and `base` must be on the target's
    /// first-parent chain.
    fn first_parent_steps(
        &self,
        base: &Revision,
        target: &Revision,
    ) -> Result<Vec<CommitStep>, GitError>;

    /// Lists the supported source entries of `revision`'s tree, sorted by raw
    /// repository path bytes.
    fn source_entries(&self, revision: &Revision) -> Result<Vec<SourceEntry>, GitError>;

    /// Reports whether `path` names a tree or blob in `revision`'s tree.
    fn path_exists(&self, revision: &Revision, path: &base::RepoPath) -> Result<bool, GitError>;

    /// Reads the exact bytes of the blob `id`.
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;
}

/// The wrapped `gix::Repository` is private; callers only observe OwnAI-owned
/// values through [`SnapshotRepository`].
pub struct GitRepository {
    repo: gix::Repository,
}

impl GitRepository {
    /// Uses `gix`'s default trust handling, so configuration is only read at the
    /// trust level the discovered repository earns; trust is never overridden to
    /// force-load unsafe configuration. Opened read-only, installing the bounded
    /// object cache from TECHNICAL_DESIGN.md section 8.4 if none is present.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, GitError> {
        let start = start.as_ref();
        let mut repo = gix::discover(start).map_err(|source| GitError::RepositoryNotFound {
            start: start.to_path_buf(),
            source: Box::new(source),
        })?;
        repo.object_cache_size_if_unset(OBJECT_CACHE_SIZE_BYTES);
        Ok(Self { repo })
    }

    /// The discovered Git directory (`.git`, or the bare repository root),
    /// reported in diagnostics.
    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    pub fn work_dir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// Lists stage-zero regular files from the index. Unmerged entries cannot
    /// identify one staged version and are deliberately excluded.
    pub fn index_source_entries(&self) -> Result<Vec<SourceEntry>, GitError> {
        let index = self
            .repo
            .index_or_empty()
            .map_err(|source| GitError::IndexRead {
                repository: self.location(),
                source: Box::new(source),
            })?;
        let mut entries = Vec::new();
        for entry in index.entries() {
            if entry.stage_raw() != 0 || !matches!(entry.mode.bits(), 0o100644 | 0o100755) {
                continue;
            }
            let raw: &[u8] = entry.path(&index).as_ref();
            let path = base::RepoPath::new(raw).map_err(|source| GitError::InvalidRepoPath {
                repository: self.location(),
                path: bstr::BString::from(raw.to_vec()),
                source,
            })?;
            if path.language().is_some() {
                entries.push(SourceEntry {
                    path,
                    blob_id: ObjectId::from_gix(&entry.id),
                    executable: entry.mode.bits() == 0o100755,
                });
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    pub(crate) fn gix(&self) -> &gix::Repository {
        &self.repo
    }

    pub(crate) fn location(&self) -> PathBuf {
        self.repo.git_dir().to_path_buf()
    }
}

impl fmt::Debug for GitRepository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitRepository")
            .field("git_dir", &self.repo.git_dir())
            .finish()
    }
}

impl SnapshotRepository for GitRepository {
    fn resolve_commit(&self, spec: &str) -> Result<Revision, GitError> {
        crate::revision::resolve_commit(self, spec)
    }

    fn first_parent_steps(
        &self,
        base: &Revision,
        target: &Revision,
    ) -> Result<Vec<CommitStep>, GitError> {
        crate::revision::first_parent_steps(self, base, target)
    }

    fn source_entries(&self, revision: &Revision) -> Result<Vec<SourceEntry>, GitError> {
        crate::tree::source_entries(self, revision)
    }

    fn path_exists(&self, revision: &Revision, path: &base::RepoPath) -> Result<bool, GitError> {
        crate::tree::path_exists(self, revision, path)
    }

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        crate::tree::read_blob(self, id)
    }
}
