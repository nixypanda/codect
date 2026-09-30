//! Elm language adapter for OwnAI.
//!
//! Owns the Elm parser constructor and all Elm grammar node-kind knowledge, and
//! implements the OwnAI language projector interface for Elm source files.

pub mod extract;
pub mod render;
pub mod syntax;

use base::{
    Language, LanguageProjector, ProjectedFile, ProjectionError, ProjectionInput, RepoPath,
};

/// Projects Elm source files into the shared projection model.
#[derive(Clone, Copy, Debug, Default)]
pub struct ElmProjector;

impl ElmProjector {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageProjector for ElmProjector {
    fn language(&self) -> Language {
        Language::Elm
    }

    fn supports_path(&self, path: &RepoPath) -> bool {
        path.is_elm()
    }

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
        extract::project_file(input)
    }
}
