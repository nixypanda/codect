//! The language adapter interface.

use crate::diagnostic::ProjectionError;
use crate::model::{Language, ProjectedFile, ProjectionMode, RepoPath, SupportedPath};

/// `source` is already validated UTF-8 (TECHNICAL_DESIGN.md section 5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionInput<'a> {
    /// The path and the language derived from its extension, so the two cannot
    /// disagree.
    pub path: &'a SupportedPath,
    pub source: &'a str,
    pub mode: ProjectionMode,
}

/// Adapters own all language meaning: grammar knowledge, extraction, and
/// canonical rendering. Core selects an adapter by path, invokes it, and
/// compares the resulting canonical text. Parsing and projection must be
/// deterministic and must not depend on the current directory, locale,
/// terminal width, wall clock, environment variables, or installed compilers.
pub trait LanguageProjector: Send + Sync {
    fn language(&self) -> Language;

    fn supports_path(&self, path: &RepoPath) -> bool;

    /// Returns a fatal [`ProjectionError`] rather than guessing around
    /// erroneous syntax or emitting a partial file.
    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError>;
}
