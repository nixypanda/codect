//! The language adapter interface.
//!
//! Owns the `ProjectionInput` and `LanguageProjector` contracts that each
//! language crate implements. Core selects an adapter by repository path.

use crate::diagnostic::ProjectionError;
use crate::model::{Language, ProjectedFile, ProjectionMode, RepoPath};

/// The borrowed inputs a language adapter needs to project one source file.
///
/// `source` is already validated UTF-8 (TECHNICAL_DESIGN.md section 5.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionInput<'a> {
    /// The repository-relative path of the source file.
    pub path: &'a RepoPath,
    /// The decoded UTF-8 source contents.
    pub source: &'a str,
    /// The projection mode to produce.
    pub mode: ProjectionMode,
}

/// A language-specific projector.
///
/// Adapters own all language meaning: grammar knowledge, extraction, and
/// canonical rendering. Core selects an adapter by path, invokes it, and
/// compares the resulting canonical text. Parsing and projection must be
/// deterministic and must not depend on the current directory, locale,
/// terminal width, wall clock, environment variables, or installed compilers.
pub trait LanguageProjector: Send + Sync {
    /// The language this adapter projects.
    fn language(&self) -> Language;

    /// Whether this adapter can project the given path.
    fn supports_path(&self, path: &RepoPath) -> bool;

    /// Projects one source file into the shared model.
    ///
    /// Returns a fatal [`ProjectionError`] rather than guessing around
    /// erroneous syntax or emitting a partial file.
    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError>;
}
