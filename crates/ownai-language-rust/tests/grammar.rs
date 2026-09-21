//! Grammar smoke and capability tests (TECHNICAL_DESIGN.md section 16.1).

use tree_sitter::Parser;

#[test]
fn rust_language_assigns_to_a_parser() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("the Rust grammar must assign to a parser");
}

#[test]
fn rust_grammar_parses_the_smallest_valid_file() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("the Rust grammar must assign to a parser");
    let tree = parser.parse("", None).expect("an empty file must parse");
    assert!(!tree.root_node().has_error());
}

#[test]
fn rust_grammar_parses_a_minimal_declaration_without_errors() {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("the Rust grammar must assign to a parser");
    let tree = parser
        .parse("pub struct S;", None)
        .expect("a minimal declaration must parse");
    assert!(!tree.root_node().has_error());
}
