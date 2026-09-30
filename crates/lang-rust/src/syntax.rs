//! Rust grammar node-kind constants.
//!
//! Tree-sitter node names are centralized here so that grammar upgrades fail
//! focused tests when node names or shapes change. Every other module in this
//! crate refers to nodes through these constants.

use base::{ProjectionError, SourceSpan, SupportedPath};
use tree_sitter::{Node, Parser, Tree};

// Visible node kinds.
pub const SOURCE_FILE: &str = "source_file";
pub const ERROR: &str = "ERROR";

pub const LINE_COMMENT: &str = "line_comment";
pub const BLOCK_COMMENT: &str = "block_comment";

pub const ATTRIBUTE_ITEM: &str = "attribute_item";
pub const INNER_ATTRIBUTE_ITEM: &str = "inner_attribute_item";
pub const ATTRIBUTE: &str = "attribute";
pub const TOKEN_TREE: &str = "token_tree";

pub const OPEN_PAREN: &str = "(";
pub const CLOSE_PAREN: &str = ")";
pub const OPEN_BRACKET: &str = "[";
pub const CLOSE_BRACKET: &str = "]";
pub const OPEN_BRACE: &str = "{";
pub const CLOSE_BRACE: &str = "}";

pub const VISIBILITY_MODIFIER: &str = "visibility_modifier";
pub const MUTABLE_SPECIFIER: &str = "mutable_specifier";

pub const MOD_ITEM: &str = "mod_item";
pub const FOREIGN_MOD_ITEM: &str = "foreign_mod_item";
pub const EXTERN_MODIFIER: &str = "extern_modifier";
pub const DECLARATION_LIST: &str = "declaration_list";

pub const STRUCT_ITEM: &str = "struct_item";
pub const UNION_ITEM: &str = "union_item";
pub const ENUM_ITEM: &str = "enum_item";
pub const TYPE_ITEM: &str = "type_item";
pub const TRAIT_ITEM: &str = "trait_item";
pub const IMPL_ITEM: &str = "impl_item";
pub const ASSOCIATED_TYPE: &str = "associated_type";

pub const FUNCTION_ITEM: &str = "function_item";
pub const FUNCTION_SIGNATURE_ITEM: &str = "function_signature_item";
pub const CONST_ITEM: &str = "const_item";
pub const STATIC_ITEM: &str = "static_item";

pub const FIELD_DECLARATION_LIST: &str = "field_declaration_list";
pub const ORDERED_FIELD_DECLARATION_LIST: &str = "ordered_field_declaration_list";
pub const FIELD_DECLARATION: &str = "field_declaration";
pub const ENUM_VARIANT_LIST: &str = "enum_variant_list";
pub const ENUM_VARIANT: &str = "enum_variant";

pub const TYPE_PARAMETERS: &str = "type_parameters";
pub const TYPE_PARAMETER: &str = "type_parameter";
pub const LIFETIME_PARAMETER: &str = "lifetime_parameter";
pub const CONST_PARAMETER: &str = "const_parameter";
pub const WHERE_CLAUSE: &str = "where_clause";
pub const TRAIT_BOUNDS: &str = "trait_bounds";

pub const PARAMETERS: &str = "parameters";
pub const PARAMETER: &str = "parameter";
pub const SELF_PARAMETER: &str = "self_parameter";
pub const VARIADIC_PARAMETER: &str = "variadic_parameter";
pub const BLOCK: &str = "block";

pub const IDENTIFIER: &str = "identifier";
pub const TYPE_IDENTIFIER: &str = "type_identifier";
pub const FIELD_IDENTIFIER: &str = "field_identifier";
pub const PRIMITIVE_TYPE: &str = "primitive_type";

pub const GENERIC_TYPE: &str = "generic_type";
pub const TUPLE_TYPE: &str = "tuple_type";
pub const REFERENCE_TYPE: &str = "reference_type";
pub const FUNCTION_TYPE: &str = "function_type";
pub const TYPE_ARGUMENTS: &str = "type_arguments";

pub const STRING_LITERAL: &str = "string_literal";
pub const RAW_STRING_LITERAL: &str = "raw_string_literal";
pub const CHAR_LITERAL: &str = "char_literal";
pub const INTEGER_LITERAL: &str = "integer_literal";
pub const FLOAT_LITERAL: &str = "float_literal";
pub const BOOLEAN_LITERAL: &str = "boolean_literal";
pub const NEGATIVE_LITERAL: &str = "negative_literal";

pub const USE_DECLARATION: &str = "use_declaration";
pub const EXTERN_CRATE_DECLARATION: &str = "extern_crate_declaration";
pub const MACRO_INVOCATION: &str = "macro_invocation";
pub const MACRO_DEFINITION: &str = "macro_definition";
pub const EMPTY_STATEMENT: &str = "empty_statement";
pub const LET_DECLARATION: &str = "let_declaration";

pub const EQUAL: &str = "=";
pub const SEMICOLON: &str = ";";

// Named-field names used during traversal.
pub const FIELD_NAME: &str = "name";
pub const FIELD_TYPE: &str = "type";
pub const FIELD_BODY: &str = "body";
pub const FIELD_TRAIT: &str = "trait";
pub const FIELD_ARGUMENTS: &str = "arguments";
pub const FIELD_TYPE_ARGUMENTS: &str = "type_arguments";
pub const FIELD_PARAMETERS: &str = "parameters";
pub const FIELD_RETURN_TYPE: &str = "return_type";
pub const FIELD_PATTERN: &str = "pattern";

/// Parses one Rust source file and rejects any tree containing `ERROR` or
/// missing nodes. `None` parse results and error nodes are fatal so that no
/// caller can emit a partial projection (TECHNICAL_DESIGN.md section 9).
pub(crate) fn parse(source: &str, path: &SupportedPath) -> Result<Tree, ProjectionError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| ProjectionError::AstInvariant {
            path: path.clone(),
            range: whole_file_span(source),
            detail: "the Rust grammar could not be assigned to a parser".to_owned(),
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ProjectionError::ParseFailed {
            path: path.clone(),
            range: whole_file_span(source),
        })?;

    if tree.root_node().has_error() {
        let range = first_error_range(tree.root_node()).unwrap_or_else(|| whole_file_span(source));
        return Err(ProjectionError::ErroneousSyntax {
            path: path.clone(),
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
