//! The language adapter interface.
//!
//! Projection is deterministic and never depends on the current directory,
//! locale, terminal width, wall clock, environment, or installed compilers, and
//! a projection failure is fatal rather than a partial file.

use crate::diagnostic::ProjectionError;
use crate::model::{Language, ProjectedFile, ProjectionMode, RepoPath, SupportedPath};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionInput<'a> {
    pub path: &'a SupportedPath,
    pub source: &'a str,
    pub mode: ProjectionMode,
}

pub trait LanguageProjector: Send + Sync {
    fn language(&self) -> Language;

    fn supports_path(&self, path: &RepoPath) -> bool;

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError>;
}
