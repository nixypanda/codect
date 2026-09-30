use base::{
    LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode, RepoPath, SupportedPath,
};
use lang_elm::ElmProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("test path is valid")).expect("supported path")
}

fn project(source: &str, mode: ProjectionMode) -> Result<String, ProjectionError> {
    let path = supported("input.elm");
    ElmProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .map(|file| file.canonical_text().to_owned())
}

#[test]
fn erroneous_syntax_is_fatal_with_a_bounded_range() {
    let source = "module A exposing (..)\n\ntype =\n";
    let error = project(source, ProjectionMode::Types).unwrap_err();

    match error {
        ProjectionError::ErroneousSyntax { range, .. } => {
            assert!(range.start_byte() <= range.end_byte());
            assert!(range.end_byte() <= source.len());
        }
        other => panic!("expected ErroneousSyntax, got {other:?}"),
    }
}

#[test]
fn a_valid_file_without_declarations_is_not_an_error() {
    assert_eq!(
        project("module A exposing (..)\n", ProjectionMode::Types).unwrap(),
        "module A\n"
    );
}

#[test]
fn colliding_declarations_get_a_source_order_ordinal_key() {
    let source =
        "module A exposing (..)\n\ntype alias Foo =\n    Int\n\ntype alias Foo =\n    String\n";
    let path = supported("input.elm");
    let file = ElmProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Types,
        })
        .expect("duplicate aliases are syntactically valid");
    let keys: Vec<&str> = file
        .items()
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect();
    assert_eq!(keys, ["A", "A type alias Foo", "A type alias Foo~1"]);
}
