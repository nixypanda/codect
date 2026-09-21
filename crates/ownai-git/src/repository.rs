//! Repository discovery and read-only opening.
//!
//! Wraps `gix::Repository` and exposes only OwnAI-owned values to callers.
//!
//! OwnAI-owned value types ([`HashKind`], [`ObjectId`], [`Revision`],
//! [`SourceEntry`]) and the [`SnapshotRepository`] trait mirror
//! TECHNICAL_DESIGN.md section 8. No `gix` type appears in a public signature.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::GitError;

/// The object cache budget for one opened repository (TECHNICAL_DESIGN.md
/// section 8.4).
///
/// The value is deliberately conservative for the MVP and is kept inside this
/// crate; there are no tuning flags. It bounds the fully decoded object cache
/// used while reading trees and blobs repeatedly.
const OBJECT_CACHE_SIZE_BYTES: usize = 8 * 1024 * 1024;

/// The hash algorithm used by a repository's object ids.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HashKind {
    /// SHA-1 object ids, the common case.
    Sha1,
    /// SHA-256 object ids.
    Sha256,
}

impl HashKind {
    /// Converts a `gix` hash kind into the OwnAI-owned value.
    ///
    /// This stays inside the crate so that `gix` types never leave it. With
    /// the approved `gix` feature set (`sha1` and `sha256`) only those two
    /// kinds can occur.
    pub(crate) fn from_gix(kind: gix::hash::Kind) -> Self {
        match kind {
            gix::hash::Kind::Sha1 => Self::Sha1,
            gix::hash::Kind::Sha256 => Self::Sha256,
            _ => Self::Sha256,
        }
    }
}

/// An OwnAI-owned, hash-algorithm-tagged Git object id.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId {
    /// The hash algorithm that produced [`Self::bytes`].
    pub kind: HashKind,
    /// The raw object id bytes: 20 for SHA-1, 32 for SHA-256.
    pub bytes: Vec<u8>,
}

impl ObjectId {
    /// Builds an OwnAI object id from a `gix` object id.
    pub(crate) fn from_gix(id: &gix::hash::ObjectId) -> Self {
        Self {
            kind: HashKind::from_gix(id.kind()),
            bytes: id.as_slice().to_vec(),
        }
    }

    /// Converts this OwnAI object id back to a `gix` object id.
    ///
    /// The length and the declared [`HashKind`] must agree; a mismatch is an
    /// [`GitError::InvalidObjectId`] rather than a panic.
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
    /// Writes the object id as lowercase hexadecimal.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.bytes {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A resolved commit revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Revision {
    /// The object id of the peeled commit.
    pub object_id: ObjectId,
}

/// A supported source file in a committed tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEntry {
    /// The repository-relative path of the file.
    pub path: ownai_core::RepoPath,
    /// The blob object id of the file's contents.
    pub blob_id: ObjectId,
    /// Whether the committed mode marks the file executable.
    pub executable: bool,
}

/// Read-only access to committed snapshots of a repository.
///
/// Implementations resolve a revision to exactly one commit, list the
/// supported source entries of a commit tree, and read a blob's exact bytes.
pub trait SnapshotRepository {
    /// Resolves `spec` to exactly one commit, peeling tags and other
    /// commit-ish objects.
    fn resolve_commit(&self, spec: &str) -> Result<Revision, GitError>;

    /// Lists the supported source entries of `revision`'s tree, sorted by raw
    /// repository path bytes.
    fn source_entries(&self, revision: &Revision) -> Result<Vec<SourceEntry>, GitError>;

    /// Reads the exact bytes of the blob `id`.
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;
}

/// A read-only repository discovered with `gix`.
///
/// The wrapped `gix::Repository` is private; callers only ever observe
/// OwnAI-owned values through [`SnapshotRepository`].
pub struct GitRepository {
    /// The wrapped repository. Never exposed.
    repo: gix::Repository,
}

impl GitRepository {
    /// Discovers and opens the repository that contains `start`.
    ///
    /// Discovery searches `start` and its ancestors for a normal repository, a
    /// bare repository, or a linked worktree, exactly as `gix` does. It uses
    /// `gix`'s default trust handling, so configuration is only read at the
    /// trust level the discovered repository earns; trust is never overridden
    /// to force-load unsafe configuration.
    ///
    /// The repository is opened read-only. The bounded object cache from
    /// TECHNICAL_DESIGN.md section 8.4 is installed if none is present.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, GitError> {
        let start = start.as_ref();
        let mut repo = gix::discover(start).map_err(|source| GitError::RepositoryNotFound {
            start: start.to_path_buf(),
            source: Box::new(source),
        })?;
        repo.object_cache_size_if_unset(OBJECT_CACHE_SIZE_BYTES);
        Ok(Self { repo })
    }

    /// The discovered Git directory (`.git`, or the bare repository root).
    ///
    /// This is the repository location reported in diagnostics.
    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    /// The work tree, if the repository has one.
    pub fn work_dir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

    /// The wrapped `gix` repository.
    pub(crate) fn gix(&self) -> &gix::Repository {
        &self.repo
    }

    /// The repository location for diagnostic context.
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

    fn source_entries(&self, revision: &Revision) -> Result<Vec<SourceEntry>, GitError> {
        crate::tree::source_entries(self, revision)
    }

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError> {
        crate::tree::read_blob(self, id)
    }
}
