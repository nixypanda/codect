use ownai_core::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use ownai_language_elm::ElmProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("test path is valid")).expect("supported path")
}

fn project(source: &str, mode: ProjectionMode) -> String {
    let path = supported("input.elm");
    ElmProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .expect("test source projects")
        .canonical_text()
        .to_owned()
}

fn both(source: &str) -> (String, String) {
    (
        project(source, ProjectionMode::Types),
        project(source, ProjectionMode::Signatures),
    )
}

#[test]
fn changing_only_a_function_body_leaves_both_projections_unchanged() {
    let before = "module A exposing (..)\n\nf : Int -> Int\nf x =\n    x + 1\n";
    let after = "module A exposing (..)\n\nf : Int -> Int\nf x =\n    x + 1000\n";
    assert_eq!(both(before), both(after));
}

#[test]
fn changing_comments_leaves_both_projections_unchanged() {
    let before = concat!(
        "module A exposing (..)\n\n",
        "-- explain the alias\n",
        "type alias T =\n    { a : Int }\n",
    );
    let after = concat!(
        "module A exposing (..)\n\n",
        "{- a block comment instead -}\n",
        "type alias T =\n    { a : Int {- inline -} }\n",
    );
    assert_eq!(both(before), both(after));
}

#[test]
fn changing_whitespace_leaves_both_projections_unchanged() {
    let before = "module A exposing (..)\n\n\ntype alias Pair =\n    ( String, Int )\n";
    let after = "module   A\nexposing (..)\ntype alias Pair = (String,Int)\n";
    assert_eq!(both(before), both(after));
}

#[test]
fn changing_a_type_changes_both_projections() {
    let before = "module A exposing (..)\n\ntype alias T =\n    Int\n";
    let after = "module A exposing (..)\n\ntype alias T =\n    String\n";
    assert_ne!(both(before), both(after));
    assert_ne!(
        project(before, ProjectionMode::Types),
        project(after, ProjectionMode::Types)
    );
    assert_ne!(
        project(before, ProjectionMode::Signatures),
        project(after, ProjectionMode::Signatures)
    );
}

#[test]
fn changing_a_function_signature_changes_only_signatures() {
    let before = "module A exposing (..)\n\nf : Int -> Int\nf x =\n    x\n";
    let after = "module A exposing (..)\n\nf : Int -> String\nf x =\n    x\n";

    assert_eq!(
        project(before, ProjectionMode::Types),
        project(after, ProjectionMode::Types)
    );
    assert_ne!(
        project(before, ProjectionMode::Signatures),
        project(after, ProjectionMode::Signatures)
    );
}

#[test]
fn adding_a_private_function_changes_only_signatures() {
    let before = "module A exposing (..)\n\nf : Int -> Int\nf x =\n    x\n";
    let after = concat!(
        "module A exposing (..)\n\n",
        "f : Int -> Int\nf x =\n    x\n\n",
        "hidden y =\n    y\n",
    );

    assert_eq!(
        project(before, ProjectionMode::Types),
        project(after, ProjectionMode::Types)
    );
    assert_ne!(
        project(before, ProjectionMode::Signatures),
        project(after, ProjectionMode::Signatures)
    );
}

#[test]
fn reordering_declarations_changes_projected_order() {
    let before = "module A exposing (..)\n\ntype alias First =\n    Int\n\ntype alias Second =\n    String\n";
    let after = "module A exposing (..)\n\ntype alias Second =\n    String\n\ntype alias First =\n    Int\n";

    assert_ne!(
        project(before, ProjectionMode::Types),
        project(after, ProjectionMode::Types)
    );
    assert!(
        project(before, ProjectionMode::Types)
            .find("First")
            .expect("First is projected")
            < project(before, ProjectionMode::Types)
                .find("Second")
                .expect("Second is projected")
    );
}

#[test]
fn unannotated_declarations_stay_visible_in_signatures() {
    let source = "module A exposing (..)\n\nnormalize x =\n    x\n";

    let signatures = project(source, ProjectionMode::Signatures);
    assert_eq!(
        signatures,
        "module A\n\nnormalize : <missing type annotation>\n"
    );
    assert!(
        !project(source, ProjectionMode::Types).contains("normalize"),
        "an unannotated value must not appear in Types mode"
    );
}
