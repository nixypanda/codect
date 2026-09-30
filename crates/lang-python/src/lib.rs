//! Python language adapter for Codect.
//!
//! Owns the Python parser constructor and all Python grammar node-kind
//! knowledge, and implements the Codect language projector interface for Python
//! source files.

pub mod extract;
pub mod render;
pub mod syntax;

use base::{
    Language, LanguageProjector, ProjectedFile, ProjectionError, ProjectionInput, RepoPath,
};

/// Projects Python source files into the shared projection model.
#[derive(Clone, Copy, Debug, Default)]
pub struct PythonProjector;

impl PythonProjector {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageProjector for PythonProjector {
    fn language(&self) -> Language {
        Language::Python
    }

    fn supports_path(&self, path: &RepoPath) -> bool {
        path.is_python()
    }

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
        extract::project_file(input)
    }
}
