//! Typed failures of the shared application layer.
//!
//! The engine never formats a user-facing report. Every variant preserves the
//! structured context a caller needs to build one, and the underlying error
//! chain survives so consumers never recover context by parsing a message.

use ownai_core::{DiagnosticContext, ProjectionError, RepoPath};
use ownai_git::GitError;

use crate::config::ConfigError;
use crate::selection::{SelectionError, SelectionGroup};

/// A failure while projecting or diffing a committed revision.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A Git operation failed. `revision` is the spec the engine was reading,
    /// filled in only when the underlying error does not name one.
    #[error("{source}")]
    Git {
        revision: Option<String>,
        #[source]
        source: GitError,
    },

    /// `.ownai.toml` could not supply areas.
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// A supported source file could not be projected. Aborts the whole
    /// operation rather than yielding a partial projection.
    ///
    /// The context is boxed so the error type stays small enough to return by
    /// value from the pipeline's hot functions.
    #[error("a supported source file could not be projected")]
    Projection {
        context: Box<DiagnosticContext>,
        #[source]
        source: ProjectionError,
    },

    /// A selected path or area names nothing in the projected revision(s).
    #[error("a selected path or area does not exist in the projected revision")]
    UnsatisfiedSelection {
        /// The revision label(s) used by the diagnostic, already formatted.
        revisions: String,
        missing: Vec<SelectionGroup>,
    },

    /// No language adapter supports an explicitly supplied path.
    ///
    /// Committed paths with unsupported extensions are silently excluded; this
    /// variant exists only for the editor-facing [`crate::project_source`]
    /// path, where the caller supplied the path as the projection context and a
    /// silent empty result would be indistinguishable from a real failure.
    #[error("no language adapter supports `{path}`")]
    UnsupportedPath { path: RepoPath },

    /// A selection could not be constructed.
    #[error(transparent)]
    Selection(#[from] SelectionError),
}

impl EngineError {
    /// Wraps a Git failure, recording the revision spec when the underlying
    /// error does not already carry one.
    pub(crate) fn git(source: GitError, revision: Option<&str>) -> Self {
        Self::Git {
            revision: revision.map(str::to_owned),
            source,
        }
    }
}

impl From<GitError> for EngineError {
    fn from(source: GitError) -> Self {
        Self::Git {
            revision: None,
            source,
        }
    }
}
