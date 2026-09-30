//! Haskell language adapter for Codect.
//!
//! Owns the Haskell parser constructor and all Haskell grammar node-kind
//! knowledge, and implements the Codect language projector interface for Haskell
//! source files.

pub mod extract;
pub mod render;
pub mod syntax;

use base::{
    Language, LanguageProjector, ProjectedFile, ProjectionError, ProjectionInput, RepoPath,
};

/// Projects Haskell source files into the shared projection model.
#[derive(Clone, Copy, Debug, Default)]
pub struct HaskellProjector;

impl HaskellProjector {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageProjector for HaskellProjector {
    fn language(&self) -> Language {
        Language::Haskell
    }

    fn supports_path(&self, path: &RepoPath) -> bool {
        path.is_haskell()
    }

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
        extract::project_file(input)
    }
}
