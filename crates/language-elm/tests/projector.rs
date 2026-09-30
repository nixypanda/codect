use base::{
    LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode, RepoPath, SupportedPath,
};
use language_elm::ElmProjector;

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
