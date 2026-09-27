//! Read-only Git access for OwnAI.
//!
//! `gix` types must never leave this crate; every public signature, error
//! variant, and value type is OwnAI-owned.

pub mod repository;
pub mod revision;
pub mod tree;

use std::error::Error;
use std::path::PathBuf;

use bstr::BString;

use ownai_core::RepoPathError;

pub use repository::{
    GitRepository, HashKind, ObjectId, Revision, SnapshotRepository, SourceEntry,
};

/// A typed failure in the read-only Git layer.
///
/// No variant exposes a `gix` type; underlying Git errors are type-erased into
/// a boxed [`std::error::Error`] so the chain survives for diagnostics without
/// leaking the implementation (TECHNICAL_DESIGN.md section 15).
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("no Git repository found starting at `{start}`")]
    RepositoryNotFound {
        start: PathBuf,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    #[error("revision `{revision}` was not found in repository `{repository}`")]
    RevisionNotFound {
        repository: PathBuf,
        revision: String,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    #[error("revision `{revision}` is ambiguous in repository `{repository}`")]
    AmbiguousRevision {
        repository: PathBuf,
        revision: String,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// The `diff` command takes two independent revision arguments, so a range
    /// passed as one argument is rejected.
    #[error("revision `{revision}` does not identify exactly one object; ranges are not supported")]
    RevisionRange {
        repository: PathBuf,
        revision: String,
    },

    #[error("revision `{revision}` resolved to {object_id}, which does not peel to a commit")]
    NotPeelable {
        repository: PathBuf,
        revision: String,
        object_id: ObjectId,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    #[error("Git object {object_id} could not be read from repository `{repository}`")]
    ObjectRead {
        repository: PathBuf,
        object_id: ObjectId,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    #[error("a commit tree in repository `{repository}` could not be traversed")]
    TreeTraversal {
        repository: PathBuf,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    #[error("committed entry `{path:?}` is not a usable repository path")]
    InvalidRepoPath {
        repository: PathBuf,
        path: BString,
        #[source]
        source: RepoPathError,
    },

    #[error("object id {object_id} is not a valid Git object id for repository `{repository}`")]
    InvalidObjectId {
        repository: PathBuf,
        object_id: ObjectId,
    },

    #[error("the Git index in repository `{repository}` could not be read")]
    IndexRead {
        repository: PathBuf,
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
}
