//! Typed library errors and user-facing diagnostics.
//!
//! Core does not depend on `miette`; presenting a report belongs to the CLI.

use std::error::Error;
use std::path::PathBuf;

use crate::model::{Language, RepoPath, SourceSpan, SupportedPath};

/// A rejected repository-relative path (TECHNICAL_DESIGN.md section 5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RepoPathError {
    #[error("repository path is empty")]
    Empty,

    #[error("repository path must be relative to the repository root")]
    Absolute,

    #[error("repository path must not contain `..` traversal components")]
    ParentTraversal,

    #[error("repository path is not normalized")]
    NotNormalized,
}

/// A fatal failure while projecting one supported source file.
///
/// Unsupported extensions, symlinks, submodules, and macro-generated
/// declarations are exclusions, not projection errors. An error here means
/// OwnAI must not emit a partial result.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// Elm, Haskell, Python, and Rust source contents must be valid UTF-8; see
    /// TECHNICAL_DESIGN.md section 5.1.
    #[error("source file `{path}` is not valid UTF-8")]
    InvalidUtf8 {
        path: SupportedPath,
        #[source]
        source: std::str::Utf8Error,
    },

    #[error("`{path}` could not be parsed as valid {language:?} source", language = .path.language())]
    ParseFailed {
        path: SupportedPath,
        range: SourceSpan,
    },

    /// The parsed tree contained `ERROR` or missing nodes.
    #[error("`{path}` contains {language:?} syntax errors", language = .path.language())]
    ErroneousSyntax {
        path: SupportedPath,
        range: SourceSpan,
    },

    #[error("`{path}` violates a {language:?} projection invariant: {detail}", language = .path.language())]
    AstInvariant {
        path: SupportedPath,
        range: SourceSpan,
        detail: String,
    },
}

impl ProjectionError {
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::InvalidUtf8 { path, .. }
            | Self::ParseFailed { path, .. }
            | Self::ErroneousSyntax { path, .. }
            | Self::AstInvariant { path, .. } => path.path(),
        }
    }

    /// The path and its derived language, as stored on the error.
    ///
    /// [`ProjectionError::path`] and [`ProjectionError::language`] are the two
    /// halves of this value; use this when a caller needs them together (for
    /// example to build a [`DiagnosticContext`]).
    pub fn supported_path(&self) -> &SupportedPath {
        match self {
            Self::InvalidUtf8 { path, .. }
            | Self::ParseFailed { path, .. }
            | Self::ErroneousSyntax { path, .. }
            | Self::AstInvariant { path, .. } => path,
        }
    }

    pub fn language(&self) -> Language {
        match self {
            Self::InvalidUtf8 { path, .. }
            | Self::ParseFailed { path, .. }
            | Self::ErroneousSyntax { path, .. }
            | Self::AstInvariant { path, .. } => path.language(),
        }
    }

    /// The source range involved in the failure, when one is meaningful. UTF-8
    /// failures carry no range because decoding fails before a coherent range
    /// exists; the underlying [`std::str::Utf8Error`] reports the valid prefix
    /// length instead.
    pub fn range(&self) -> Option<&SourceSpan> {
        match self {
            Self::InvalidUtf8 { .. } => None,
            Self::ParseFailed { range, .. }
            | Self::ErroneousSyntax { range, .. }
            | Self::AstInvariant { range, .. } => Some(range),
        }
    }
}

/// Maps invalid UTF-8 to [`ProjectionError::InvalidUtf8`] because a supported
/// source blob must be valid UTF-8.
pub fn decode_source<'a>(
    path: &SupportedPath,
    bytes: &'a [u8],
) -> Result<&'a str, ProjectionError> {
    std::str::from_utf8(bytes).map_err(|source| ProjectionError::InvalidUtf8 {
        path: path.clone(),
        source,
    })
}

/// Structured context attached to a diagnostic report (TECHNICAL_DESIGN.md
/// section 15).
///
/// Every field is optional because a given failure may not have all of them.
/// Values are kept structured rather than preformatted so the CLI can decide
/// how to render them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiagnosticContext {
    pub repository: Option<PathBuf>,
    pub revision: Option<String>,
    /// The file location, when the failure has one. A range implies a path.
    pub location: Option<Location>,
}

/// The source file a failure occurred in, and the range within it when one is
/// meaningful.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Location {
    pub path: SupportedPath,
    pub range: Option<SourceSpan>,
}

/// A structured diagnostic: context plus an underlying error chain.
///
/// The CLI converts this value into its own user-facing report.
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct Diagnostic {
    pub context: DiagnosticContext,
    #[source]
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl Diagnostic {
    pub fn new(context: DiagnosticContext, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            context,
            source: Box::new(source),
        }
    }

    pub fn source_error(&self) -> &(dyn Error + Send + Sync + 'static) {
        self.source.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    #[test]
    fn decode_source_reports_invalid_utf8_with_context() {
        let path = supported("src/lib.rs");
        let error = decode_source(&path, b"pub fn main() { \xFF }").unwrap_err();

        assert_eq!(error.path(), path.path());
        assert_eq!(error.language(), Language::Rust);
        assert!(error.range().is_none());
        assert!(matches!(error, ProjectionError::InvalidUtf8 { .. }));
    }

    #[test]
    fn diagnostic_preserves_the_error_chain() {
        let path = supported("src/lib.rs");
        let error = decode_source(&path, b"\xFF").unwrap_err();
        let diagnostic = Diagnostic::new(
            DiagnosticContext {
                revision: Some("HEAD".to_owned()),
                location: Some(Location { path, range: None }),
                ..DiagnosticContext::default()
            },
            error,
        );

        assert_eq!(diagnostic.context.revision.as_deref(), Some("HEAD"));
        assert!(diagnostic.source_error().source().is_some());
    }
}
