//! Repository discovery and read-only opening.
//!
//! Codect-owned value types and the [`SnapshotRepository`] trait mirror
//! TECHNICAL_DESIGN.md section 8; no `gix` type appears in a public signature.
//!
//! Trait inputs are already-resolved commits: `first_parent_steps` requires
//! `base` on `target`'s first-parent chain, and `merge_base` reports a typed
//! [`GitError::NoMergeBase`] when the commits share no ancestor. Index and
//! untracked listings cover only stage-zero entries and untracked, non-ignored
//! regular files with a supported language.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::GitError;

const OBJECT_CACHE_SIZE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HashKind {
    Sha1,
    Sha256,
}

impl HashKind {
    pub(crate) fn from_gix(kind: gix::hash::Kind, repository: &Path) -> Result<Self, GitError> {
        match kind {
            gix::hash::Kind::Sha1 => Ok(Self::Sha1),
            gix::hash::Kind::Sha256 => Ok(Self::Sha256),
            _ => Err(GitError::UnsupportedHashKind {
                repository: repository.to_path_buf(),
                kind: format!("{kind:?}"),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId {
    pub kind: HashKind,
    pub bytes: Vec<u8>,
}

impl ObjectId {
    pub(crate) fn from_gix(id: &gix::hash::ObjectId, repository: &Path) -> Result<Self, GitError> {
        Ok(Self {
            kind: HashKind::from_gix(id.kind(), repository)?,
            bytes: id.as_slice().to_vec(),
        })
    }

    pub(crate) fn to_gix(&self, repository: &Path) -> Result<gix::hash::ObjectId, GitError> {
        let id = gix::hash::ObjectId::try_from(self.bytes.as_slice()).map_err(|_| {
            GitError::InvalidObjectId {
                repository: repository.to_path_buf(),
                object_id: self.clone(),
            }
        })?;

        if HashKind::from_gix(id.kind(), repository)? != self.kind {
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

pub trait SnapshotRepository {
    fn resolve_commit(&self, spec: &str) -> Result<Revision, GitError>;

    fn first_parent_steps(
        &self,
        base: &Revision,
        target: &Revision,
    ) -> Result<Vec<CommitStep>, GitError>;

    fn merge_base(&self, one: &Revision, two: &Revision) -> Result<Revision, GitError>;

    fn source_entries(&self, revision: &Revision) -> Result<Vec<SourceEntry>, GitError>;

    fn path_exists(&self, revision: &Revision, path: &base::RepoPath) -> Result<bool, GitError>;

    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;

    fn untracked_paths(&self) -> Result<Vec<base::RepoPath>, GitError>;
}

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

    pub fn git_dir(&self) -> &Path {
        self.repo.git_dir()
    }

    pub fn work_dir(&self) -> Option<&Path> {
        self.repo.workdir()
    }

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
                    blob_id: ObjectId::from_gix(&entry.id, &self.location())?,
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

    fn merge_base(&self, one: &Revision, two: &Revision) -> Result<Revision, GitError> {
        crate::revision::merge_base(self, one, two)
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

    fn untracked_paths(&self) -> Result<Vec<base::RepoPath>, GitError> {
        crate::worktree::untracked_paths(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_hash_kinds_map_without_an_error() {
        let repository = Path::new("/repo");
        assert_eq!(
            HashKind::from_gix(gix::hash::Kind::Sha1, repository).expect("sha1 is supported"),
            HashKind::Sha1
        );
        assert_eq!(
            HashKind::from_gix(gix::hash::Kind::Sha256, repository).expect("sha256 is supported"),
            HashKind::Sha256
        );
    }
}
