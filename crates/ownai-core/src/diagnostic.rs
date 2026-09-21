//! Typed library errors and user-facing diagnostics.
//!
//! Library crates use `thiserror` to define typed errors. Diagnostics carry
//! repository location, revision string, source path, language, source range,
//! and the underlying error chain where applicable.
//!
//! `ownai-core` models that context as structured fields. It does not depend on
//! `miette`; presenting a report for a person belongs to the CLI layer.

use std::error::Error;
use std::path::PathBuf;

use crate::model::{Language, RepoPath, SourceSpan};

/// A rejected repository-relative path (TECHNICAL_DESIGN.md section 5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RepoPathError {
    /// The path was empty.
    #[error("repository path is empty")]
    Empty,

    /// The path was absolute rather than repository-relative.
    #[error("repository path must be relative to the repository root")]
    Absolute,

    /// The path contained a `..` component that could escape the repository.
    #[error("repository path must not contain `..` traversal components")]
    ParentTraversal,

    /// The path was not in normalized repository form.
    ///
    /// This covers empty components (`//` or a trailing `/`), `.` components,
    /// and NUL bytes.
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
    /// A supported source blob is not valid UTF-8.
    ///
    /// Elm and Rust source contents must be valid UTF-8; see
    /// TECHNICAL_DESIGN.md section 5.1.
    #[error("source file `{path}` is not valid UTF-8")]
    InvalidUtf8 {
        /// The repository-relative source path.
        path: RepoPath,
        /// The language implied by the source path.
        language: Language,
        /// The underlying UTF-8 validation error.
        #[source]
        source: std::str::Utf8Error,
    },

    /// The parser could not produce a syntax tree at all.
    #[error("`{path}` could not be parsed as valid {language:?} source")]
    ParseFailed {
        /// The repository-relative source path.
        path: RepoPath,
        /// The language that was parsed.
        language: Language,
        /// A useful source range within the failing file.
        range: SourceSpan,
    },

    /// The parsed tree contained `ERROR` or missing nodes.
    #[error("`{path}` contains {language:?} syntax errors")]
    ErroneousSyntax {
        /// The repository-relative source path.
        path: RepoPath,
        /// The language that was parsed.
        language: Language,
        /// The first useful error range in the source.
        range: SourceSpan,
    },

    /// An adapter found an AST shape that violates its invariants.
    #[error("`{path}` violates a {language:?} projection invariant: {detail}")]
    AstInvariant {
        /// The repository-relative source path.
        path: RepoPath,
        /// The language being projected.
        language: Language,
        /// The source range of the offending node.
        range: SourceSpan,
        /// A short, structured description of the violated invariant.
        detail: String,
    },
}

impl ProjectionError {
    /// The repository-relative source path involved in the failure.
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::InvalidUtf8 { path, .. }
            | Self::ParseFailed { path, .. }
            | Self::ErroneousSyntax { path, .. }
            | Self::AstInvariant { path, .. } => path,
        }
    }

    /// The language involved in the failure.
    pub fn language(&self) -> Language {
        match self {
            Self::InvalidUtf8 { language, .. }
            | Self::ParseFailed { language, .. }
            | Self::ErroneousSyntax { language, .. }
            | Self::AstInvariant { language, .. } => *language,
        }
    }

    /// The source range involved in the failure, when one is meaningful.
    ///
    /// UTF-8 failures carry no range because decoding fails before a coherent
    /// range exists; the underlying [`std::str::Utf8Error`] reports the valid
    /// prefix length instead.
    pub fn range(&self) -> Option<&SourceSpan> {
        match self {
            Self::InvalidUtf8 { .. } => None,
            Self::ParseFailed { range, .. }
            | Self::ErroneousSyntax { range, .. }
            | Self::AstInvariant { range, .. } => Some(range),
        }
    }
}

/// Decodes committed source bytes as UTF-8.
///
/// A supported source blob containing invalid UTF-8 is a fatal projection
/// diagnostic, so this maps the failure to [`ProjectionError::InvalidUtf8`]
/// with its path and language context. The caller supplies the language that
/// the path selects.
pub fn decode_source<'a>(
    path: &RepoPath,
    language: Language,
    bytes: &'a [u8],
) -> Result<&'a str, ProjectionError> {
    std::str::from_utf8(bytes).map_err(|source| ProjectionError::InvalidUtf8 {
        path: path.clone(),
        language,
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
    /// The discovered repository location, if a repository was opened.
    pub repository: Option<PathBuf>,
    /// The revision string the user supplied, if any.
    pub revision: Option<String>,
    /// The repository-relative source path, if the failure names one.
    pub path: Option<RepoPath>,
    /// The language involved, if the failure names one.
    pub language: Option<Language>,
    /// The source range involved, if the failure names one.
    pub range: Option<SourceSpan>,
}

/// A structured diagnostic: context plus an underlying error chain.
///
/// `miette` rendering is intentionally absent from core. The CLI converts this
/// value into its own user-facing report.
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct Diagnostic {
    /// The structured context for the report.
    pub context: DiagnosticContext,
    /// The underlying error, preserving its source chain.
    #[source]
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl Diagnostic {
    /// Wraps an underlying error with structured diagnostic context.
    pub fn new(context: DiagnosticContext, source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            context,
            source: Box::new(source),
        }
    }

    /// The underlying error.
    pub fn source_error(&self) -> &(dyn Error + Send + Sync + 'static) {
        self.source.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_source_accepts_valid_utf8() {
        let path = RepoPath::new("src/lib.rs").unwrap();
        let decoded = decode_source(&path, Language::Rust, b"pub fn main() {}").unwrap();
        assert_eq!(decoded, "pub fn main() {}");
    }

    #[test]
    fn decode_source_reports_invalid_utf8_with_context() {
        let path = RepoPath::new("src/lib.rs").unwrap();
        let error = decode_source(&path, Language::Rust, b"pub fn main() { \xFF }").unwrap_err();

        assert_eq!(error.path(), &path);
        assert_eq!(error.language(), Language::Rust);
        assert!(error.range().is_none());
        assert!(matches!(error, ProjectionError::InvalidUtf8 { .. }));
    }

    #[test]
    fn diagnostic_preserves_the_error_chain() {
        let path = RepoPath::new("src/lib.rs").unwrap();
        let error = decode_source(&path, Language::Rust, b"\xFF").unwrap_err();
        let diagnostic = Diagnostic::new(
            DiagnosticContext {
                revision: Some("HEAD".to_owned()),
                path: Some(path),
                language: Some(Language::Rust),
                ..DiagnosticContext::default()
            },
            error,
        );

        assert_eq!(diagnostic.context.revision.as_deref(), Some("HEAD"));
        assert!(diagnostic.source_error().source().is_some());
    }
}
