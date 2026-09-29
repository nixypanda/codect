//! Python grammar node-kind constants.
//!
//! Tree-sitter node names are centralized here so that grammar upgrades fail
//! focused tests when node names or shapes change.
//!
//! Python is whitespace-sensitive, but the grammar normalizes indentation into
//! `block` nodes, so extraction never consults raw indentation (except when a
//! `block` is reconstructed, where four spaces per level are emitted).

use ownai_core::{Language, ProjectionError, RepoPath, SourceSpan};
use tree_sitter::{Node, Parser};

// Visible node kinds.
pub const MODULE: &str = "module";
pub const CLASS_DEFINITION: &str = "class_definition";
pub const FUNCTION_DEFINITION: &str = "function_definition";
pub const DECORATED_DEFINITION: &str = "decorated_definition";
pub const DECORATOR: &str = "decorator";
pub const BLOCK: &str = "block";
pub const EXPRESSION_STATEMENT: &str = "expression_statement";
pub const ASSIGNMENT: &str = "assignment";
pub const AUGMENTED_ASSIGNMENT: &str = "augmented_assignment";
pub const TYPE_ALIAS_STATEMENT: &str = "type_alias_statement";
pub const COMMENT: &str = "comment";
pub const STRING: &str = "string";
pub const IDENTIFIER: &str = "identifier";
pub const TYPE: &str = "type";
pub const GENERIC_TYPE: &str = "generic_type";
pub const BINARY_OPERATOR: &str = "binary_operator";
pub const TYPE_PARAMETER: &str = "type_parameter";
pub const PARAMETERS: &str = "parameters";
pub const TYPED_PARAMETER: &str = "typed_parameter";
pub const DEFAULT_PARAMETER: &str = "default_parameter";
pub const TYPED_DEFAULT_PARAMETER: &str = "typed_default_parameter";
pub const LIST_SPLAT_PATTERN: &str = "list_splat_pattern";
pub const DICTIONARY_SPLAT_PATTERN: &str = "dictionary_splat_pattern";
pub const TUPLE_PATTERN: &str = "tuple_pattern";
pub const KEYWORD_SEPARATOR: &str = "keyword_separator";
pub const KEYWORD_ARGUMENT: &str = "keyword_argument";
pub const POSITIONAL_SEPARATOR: &str = "positional_separator";
pub const PASS_STATEMENT: &str = "pass_statement";
pub const CALL: &str = "call";
pub const ATTRIBUTE: &str = "attribute";
pub const DOTTED_NAME: &str = "dotted_name";
pub const ARGUMENT_LIST: &str = "argument_list";
pub const ELIPSIS: &str = "ellipsis";

// Named-field names used during traversal.
pub const FIELD_NAME: &str = "name";
pub const FIELD_BODY: &str = "body";
pub const FIELD_SUPERCLASSES: &str = "superclasses";
pub const FIELD_TYPE_PARAMETERS: &str = "type_parameters";
pub const FIELD_PARAMETERS: &str = "parameters";
pub const FIELD_RETURN_TYPE: &str = "return_type";
pub const FIELD_DEFINITION: &str = "definition";
pub const FIELD_LEFT: &str = "left";
pub const FIELD_RIGHT: &str = "right";
pub const FIELD_TYPE: &str = "type";
pub const FIELD_VALUE: &str = "value";

/// Parses one Python source file and rejects any tree containing `ERROR` or
/// missing nodes. `None` parse results and error nodes are fatal so that no
/// caller can emit a partial projection (TECHNICAL_DESIGN.md section 9).
pub(crate) fn parse(source: &str, path: &RepoPath) -> Result<tree_sitter::Tree, ProjectionError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|_| ProjectionError::AstInvariant {
            path: path.clone(),
            language: Language::Python,
            range: whole_file_span(source),
            detail: "the Python grammar could not be assigned to a parser".to_owned(),
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ProjectionError::ParseFailed {
            path: path.clone(),
            language: Language::Python,
            range: whole_file_span(source),
        })?;

    if tree.root_node().has_error() {
        let range = first_error_range(tree.root_node()).unwrap_or_else(|| whole_file_span(source));
        return Err(ProjectionError::ErroneousSyntax {
            path: path.clone(),
            language: Language::Python,
            range,
        });
    }

    Ok(tree)
}

/// The first `ERROR` or missing node in source order, walked iteratively so the
/// cost is independent of expression depth.
pub(crate) fn first_error_range(root: Node<'_>) -> Option<SourceSpan> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            return Some(node_span(node));
        }
        let mut cursor = node.walk();
        let mut children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        while let Some(child) = children.pop() {
            stack.push(child);
        }
    }
    None
}

pub(crate) fn node_span(node: Node<'_>) -> SourceSpan {
    let start = node.start_position();
    let end = node.end_position();
    SourceSpan::new(
        node.start_byte(),
        node.end_byte(),
        start.row,
        start.column,
        end.row,
        end.column,
    )
}

pub(crate) fn whole_file_span(source: &str) -> SourceSpan {
    let mut line = 0;
    let mut column = 0;
    for character in source.chars() {
        if character == '\n' {
            line += 1;
            column = 0;
        } else {
            column += character.len_utf8();
        }
    }

    SourceSpan::new(0, source.len(), 0, 0, line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_node_kinds_still_exist() {
        let node_kinds = [
            MODULE,
            CLASS_DEFINITION,
            FUNCTION_DEFINITION,
            DECORATED_DEFINITION,
            DECORATOR,
            BLOCK,
            EXPRESSION_STATEMENT,
            ASSIGNMENT,
            TYPE_ALIAS_STATEMENT,
            COMMENT,
            STRING,
            IDENTIFIER,
            TYPE,
            GENERIC_TYPE,
            BINARY_OPERATOR,
            TYPE_PARAMETER,
            PARAMETERS,
            TYPED_PARAMETER,
            DEFAULT_PARAMETER,
            TYPED_DEFAULT_PARAMETER,
            LIST_SPLAT_PATTERN,
            DICTIONARY_SPLAT_PATTERN,
            TUPLE_PATTERN,
            KEYWORD_SEPARATOR,
            POSITIONAL_SEPARATOR,
            PASS_STATEMENT,
            CALL,
            ATTRIBUTE,
            DOTTED_NAME,
            ARGUMENT_LIST,
        ];

        for kind in node_kinds {
            assert!(
                tree_sitter_python::NODE_TYPES.contains(&format!("\"{kind}\"")),
                "grammar no longer defines the `{kind}` node kind"
            );
        }
    }

    #[test]
    fn grammar_assigns_and_parses() {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("the Python grammar must assign to a parser");
        let tree = parser.parse("", None).expect("an empty file must parse");
        assert!(!tree.root_node().has_error());
    }
}
