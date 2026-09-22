//! Adapter identity and boundary tests.

use ownai_core::{
    ItemKind, Language, LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode,
    RepoPath,
};
use ownai_language_haskell::HaskellProjector;

fn project(source: &str, mode: ProjectionMode) -> ownai_core::ProjectedFile {
    let path = RepoPath::new("src/Sample.hs").unwrap();
    HaskellProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .unwrap()
}

#[test]
fn reports_haskell_language_and_supported_paths() {
    assert_eq!(HaskellProjector.language(), Language::Haskell);
    assert!(HaskellProjector.supports_path(&RepoPath::new("src/Main.hs").unwrap()));
    assert!(!HaskellProjector.supports_path(&RepoPath::new("src/Main.lhs").unwrap()));
    assert!(!HaskellProjector.supports_path(&RepoPath::new("src/lib.rs").unwrap()));
    assert!(!HaskellProjector.supports_path(&RepoPath::new("README.md").unwrap()));
}

#[test]
fn empty_source_projects_to_empty_text() {
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let file = project("", mode);
        assert!(file.items().is_empty());
        assert_eq!(file.canonical_text(), "");
    }
}

#[test]
fn module_header_omits_the_export_list() {
    let source =
        "module Data.User (User(..), mkUser) where\n\nmkUser :: Int -> User\nmkUser = id\n";
    let file = project(source, ProjectionMode::Signatures);
    assert!(file.canonical_text().starts_with("module Data.User\n"));
    assert!(!file.canonical_text().contains("User(..)"));
    let kinds: Vec<ItemKind> = file.items().iter().map(|item| item.kind).collect();
    assert_eq!(
        kinds,
        vec![ItemKind::Module, ItemKind::Function],
        "module plus signature"
    );
}

#[test]
fn nested_class_members_are_not_emitted_separately() {
    let source = "class C a where\n  m :: a -> Int\n";
    let file = project(source, ProjectionMode::Signatures);
    assert_eq!(
        file.canonical_text(),
        "class C a where\n    m :: a -> Int\n"
    );
    assert_eq!(file.items().len(), 2);
    assert_eq!(
        file.items()[1].parent_key.as_deref(),
        Some("src/Sample.hs::class::C")
    );
}

#[test]
fn erroneous_source_is_fatal() {
    let path = RepoPath::new("src/Sample.hs").unwrap();
    let error = HaskellProjector
        .project(ProjectionInput {
            path: &path,
            source: "module =\n",
            mode: ProjectionMode::Signatures,
        })
        .expect_err("erroneous syntax must fail");
    assert!(matches!(error, ProjectionError::ErroneousSyntax { .. }));
}
