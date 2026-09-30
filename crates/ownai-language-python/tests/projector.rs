//! Adapter identity and boundary tests.

use ownai_core::{
    ItemKind, Language, LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode,
    RepoPath, SupportedPath,
};
use ownai_language_python::PythonProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
}

fn project(source: &str, mode: ProjectionMode) -> ownai_core::ProjectedFile {
    let path = supported("src/sample.py");
    PythonProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .unwrap()
}

#[test]
fn reports_python_language_and_supported_paths() {
    assert_eq!(PythonProjector.language(), Language::Python);
    assert!(PythonProjector.supports_path(&RepoPath::new("src/app.py").unwrap()));
    assert!(PythonProjector.supports_path(&RepoPath::new("src/app.pyi").unwrap()));
    assert!(!PythonProjector.supports_path(&RepoPath::new("src/lib.rs").unwrap()));
    assert!(!PythonProjector.supports_path(&RepoPath::new("README.md").unwrap()));
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
fn nested_members_are_not_emitted_separately() {
    let file = project("class Box:\n    value: int\n", ProjectionMode::Types);
    assert_eq!(file.canonical_text(), "class Box:\n    value: int\n");
    let kinds: Vec<ItemKind> = file.items().iter().map(|item| item.kind).collect();
    assert_eq!(kinds, vec![ItemKind::Type, ItemKind::Field]);
    assert_eq!(
        file.items()[1].parent_key.as_deref(),
        Some("src/sample.py::class::Box")
    );
}

#[test]
fn erroneous_source_is_fatal() {
    let path = supported("src/sample.py");
    let error = PythonProjector
        .project(ProjectionInput {
            path: &path,
            source: "class Broken(\n",
            mode: ProjectionMode::Signatures,
        })
        .expect_err("erroneous syntax must fail");
    assert!(matches!(error, ProjectionError::ErroneousSyntax { .. }));
}
