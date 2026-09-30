//! Rust language adapter for OwnAI.
//!
//! Owns the Rust parser constructor and all Rust grammar node-kind knowledge,
//! and implements the OwnAI language projector interface for Rust source files.

use ownai_core::{
    Language, LanguageProjector, ProjectedFile, ProjectionError, ProjectionInput, RepoPath,
};

pub mod extract;
pub mod render;
pub mod syntax;

/// Projects Rust source files into the shared projection model.
#[derive(Clone, Copy, Debug, Default)]
pub struct RustProjector;

impl LanguageProjector for RustProjector {
    fn language(&self) -> Language {
        Language::Rust
    }

    fn supports_path(&self, path: &RepoPath) -> bool {
        path.is_rust()
    }

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
        extract::project(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ownai_core::{ItemKind, ProjectionMode, SupportedPath};

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    fn project(source: &str, mode: ProjectionMode) -> ProjectedFile {
        let path = supported("src/lib.rs");
        RustProjector
            .project(ProjectionInput {
                path: &path,
                source,
                mode,
            })
            .unwrap()
    }

    #[test]
    fn erroneous_source_is_fatal() {
        let path = supported("src/lib.rs");
        for source in ["pub struct Broken {", "pub struct Truncated", "fn f( {"] {
            let error = RustProjector
                .project(ProjectionInput {
                    path: &path,
                    source,
                    mode: ProjectionMode::Types,
                })
                .expect_err("erroneous syntax must fail");
            assert!(
                matches!(error, ProjectionError::ErroneousSyntax { .. }),
                "{source:?} produced {error:?}"
            );
            assert!(error.range().is_some());
        }
    }

    #[test]
    fn inline_module_frames_nested_types() {
        let file = project("mod outer { pub struct Inner; }", ProjectionMode::Types);
        assert_eq!(
            file.canonical_text(),
            "mod outer {\n    pub struct Inner;\n}\n"
        );
        let kinds: Vec<ItemKind> = file.items().iter().map(|item| item.kind).collect();
        assert_eq!(kinds, vec![ItemKind::Module, ItemKind::Type]);
        assert_eq!(
            file.items()[1].parent_key.as_deref(),
            Some("src/lib.rs::mod::outer")
        );
    }
}
