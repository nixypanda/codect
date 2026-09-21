use tree_sitter::Parser;

/// Grammar upgrades must keep the Elm grammar assignable to a native parser
/// (TECHNICAL_DESIGN.md sections 4.2 and 16.1).
#[test]
fn elm_grammar_assigns_to_a_parser() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_elm::LANGUAGE.into())
        .expect("the Elm grammar must assign to a parser");
}

#[test]
fn elm_parser_accepts_the_smallest_valid_file() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_elm::LANGUAGE.into())
        .expect("the Elm grammar must assign to a parser");

    let tree = parser
        .parse("module A exposing (..)\n", None)
        .expect("the smallest valid Elm file must parse");
    assert!(
        !tree.root_node().has_error(),
        "the smallest valid Elm file must have no error nodes"
    );
}
