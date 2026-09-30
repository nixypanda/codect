//! Rust language adapter for Codect.
//!
//! Owns the Rust parser constructor and all Rust grammar node-kind knowledge,
//! and implements the Codect language projector interface for Rust source files.

pub mod extract;
pub mod render;
pub mod syntax;

use base::{
    Language, LanguageProjector, ProjectedFile, ProjectionError, ProjectionInput, RepoPath,
};

/// Projects Rust source files into the shared projection model.
#[derive(Clone, Copy, Debug, Default)]
pub struct RustProjector;

impl RustProjector {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageProjector for RustProjector {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn supports_path(&self, path: &RepoPath) -> bool {
        path.is_rust()
    }

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
        extract::project_file(input)
    }
}
