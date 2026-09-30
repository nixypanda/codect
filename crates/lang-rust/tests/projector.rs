use base::{
    ItemKind, LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode, RepoPath,
    SupportedPath,
};
use lang_rust::RustProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
}

fn project(source: &str, mode: ProjectionMode) -> base::ProjectedFile {
    let path = supported("src/lib.rs");
    RustProjector::new()
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
        let error = RustProjector::new()
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
