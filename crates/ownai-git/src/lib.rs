//! Read-only Git access for OwnAI.
//!
//! This crate owns repository discovery, revision resolution, commit peeling,
//! tree traversal, and blob reads. It presents OwnAI-owned values through the
//! [`SnapshotRepository`] interface.
//!
//! `gix` types must never leave this crate. Every public signature, error
//! variant, and value type is OwnAI-owned so that callers never depend on the
//! Git implementation.

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
/// The variants carry the diagnostic context from TECHNICAL_DESIGN.md section
/// 15 where it applies: the repository location, the user-provided revision
/// string, the offending repository-relative path, and the underlying error
/// chain through [`std::error::Error::source`].
///
/// No variant exposes a `gix` type; underlying Git errors are type-erased into
/// a boxed [`std::error::Error`] so the chain survives for diagnostics without
/// leaking the implementation.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// No Git repository could be discovered starting at the given path.
    #[error("no Git repository found starting at `{start}`")]
    RepositoryNotFound {
        /// The directory discovery started from.
        start: PathBuf,
        /// The underlying discovery error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// The revision string could not be resolved to any object.
    #[error("revision `{revision}` was not found in repository `{repository}`")]
    RevisionNotFound {
        /// The discovered repository location.
        repository: PathBuf,
        /// The user-provided revision string.
        revision: String,
        /// The underlying parsing error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// The abbreviated revision matched more than one object.
    #[error("revision `{revision}` is ambiguous in repository `{repository}`")]
    AmbiguousRevision {
        /// The discovered repository location.
        repository: PathBuf,
        /// The user-provided revision string.
        revision: String,
        /// The underlying disambiguation error, including candidates.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// The revision identified more than one object, such as a `A..B` range.
    ///
    /// The `diff` command takes two independent revision arguments, so a range
    /// passed as one argument is rejected.
    #[error("revision `{revision}` does not identify exactly one object; ranges are not supported")]
    RevisionRange {
        /// The discovered repository location.
        repository: PathBuf,
        /// The user-provided revision string.
        revision: String,
    },

    /// The revision resolved to an object that cannot peel to a commit.
    ///
    /// A tree or blob object has no commit to traverse.
    #[error("revision `{revision}` resolved to {object_id}, which does not peel to a commit")]
    NotPeelable {
        /// The discovered repository location.
        repository: PathBuf,
        /// The user-provided revision string.
        revision: String,
        /// The resolved object id that could not peel to a commit.
        object_id: ObjectId,
        /// The underlying peeling error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A Git object could not be found or decoded.
    #[error("Git object {object_id} could not be read from repository `{repository}`")]
    ObjectRead {
        /// The discovered repository location.
        repository: PathBuf,
        /// The object id the caller asked for.
        object_id: ObjectId,
        /// The underlying object lookup or decode error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A commit tree could not be traversed.
    #[error("a commit tree in repository `{repository}` could not be traversed")]
    TreeTraversal {
        /// The discovered repository location.
        repository: PathBuf,
        /// The underlying tree lookup or decode error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A committed tree entry could not be represented as a repository path.
    #[error("committed entry `{path:?}` is not a usable repository path")]
    InvalidRepoPath {
        /// The discovered repository location.
        repository: PathBuf,
        /// The raw committed path bytes that were rejected.
        path: BString,
        /// The underlying path validation error.
        #[source]
        source: RepoPathError,
    },

    /// An OwnAI object id could not be converted back to a Git object id.
    #[error("object id {object_id} is not a valid Git object id for repository `{repository}`")]
    InvalidObjectId {
        /// The discovered repository location.
        repository: PathBuf,
        /// The rejected OwnAI object id.
        object_id: ObjectId,
    },
}
