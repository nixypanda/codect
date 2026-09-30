//! Adapter identity and boundary tests.

use base::{
    ItemKind, LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode, RepoPath,
    SupportedPath,
};
use language_python::PythonProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
}

fn project(source: &str, mode: ProjectionMode) -> base::ProjectedFile {
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
