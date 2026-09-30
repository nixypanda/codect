//! Elm grammar node-kind constants.
//!
//! Tree-sitter node names are centralized here so that grammar upgrades fail
//! focused tests when node names or shapes change.

use ownai_core::{ProjectionError, SourceSpan, SupportedPath};
use tree_sitter::{Node, Parser, Tree};

// Visible node kinds.
pub const FILE: &str = "file";
pub const MODULE_DECLARATION: &str = "module_declaration";
pub const PORT: &str = "port";
pub const MODULE: &str = "module";
pub const IMPORT_CLAUSE: &str = "import_clause";
pub const EXPOSING_LIST: &str = "exposing_list";
pub const UPPER_CASE_QID: &str = "upper_case_qid";
pub const UPPER_CASE_IDENTIFIER: &str = "upper_case_identifier";
pub const LOWER_CASE_IDENTIFIER: &str = "lower_case_identifier";
pub const VALUE_QID: &str = "value_qid";
pub const DOT: &str = "dot";
pub const TYPE_DECLARATION: &str = "type_declaration";
pub const TYPE_ALIAS_DECLARATION: &str = "type_alias_declaration";
pub const TYPE_ANNOTATION: &str = "type_annotation";
pub const PORT_ANNOTATION: &str = "port_annotation";
pub const INFIX_DECLARATION: &str = "infix_declaration";
pub const VALUE_DECLARATION: &str = "value_declaration";
pub const FUNCTION_DECLARATION_LEFT: &str = "function_declaration_left";
pub const UNION_VARIANT: &str = "union_variant";
pub const LOWER_TYPE_NAME: &str = "lower_type_name";
pub const TYPE_EXPRESSION: &str = "type_expression";
pub const TYPE_REF: &str = "type_ref";
pub const TYPE_VARIABLE: &str = "type_variable";
pub const RECORD_TYPE: &str = "record_type";
pub const FIELD_TYPE: &str = "field_type";
pub const TUPLE_TYPE: &str = "tuple_type";
pub const UNIT_EXPR: &str = "unit_expr";
pub const ARROW: &str = "arrow";
pub const OPERATOR_IDENTIFIER: &str = "operator_identifier";
pub const VALUE_EXPR: &str = "value_expr";
pub const NUMBER_LITERAL: &str = "number_literal";

// Named-field names used during traversal.
pub const FIELD_NAME: &str = "name";
pub const FIELD_MODULE_NAME: &str = "moduleName";
pub const FIELD_TYPE_NAME: &str = "typeName";
pub const FIELD_TYPE_VARIABLE: &str = "typeVariable";
pub const FIELD_TYPE_EXPRESSION: &str = "typeExpression";
pub const FIELD_UNION_VARIANT: &str = "unionVariant";
pub const FIELD_PART: &str = "part";
pub const FIELD_PATTERN: &str = "pattern";
pub const FIELD_FUNCTION_LEFT: &str = "functionDeclarationLeft";
pub const FIELD_FIELD_TYPE: &str = "fieldType";
pub const FIELD_BASE_RECORD: &str = "baseRecord";
pub const FIELD_UNIT_EXPR: &str = "unitExpr";
pub const FIELD_ASSOCIATIVITY: &str = "associativity";
pub const FIELD_PRECEDENCE: &str = "precedence";
pub const FIELD_OPERATOR: &str = "operator";

/// Parses one Elm source file and rejects any tree containing `ERROR` or
/// missing nodes. `None` parse results and error nodes are fatal so that no
/// caller can emit a partial projection (TECHNICAL_DESIGN.md section 9).
pub(crate) fn parse(source: &str, path: &SupportedPath) -> Result<Tree, ProjectionError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_elm::LANGUAGE.into())
        .map_err(|_| ProjectionError::AstInvariant {
            path: path.clone(),
            range: whole_file_span(source),
            detail: "the Elm grammar could not be assigned to a parser".to_owned(),
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

/// `has_error` guarantees at least one node is an `ERROR` or missing node, but
/// the first in source order is not discoverable without walking; this returns
/// it iteratively to keep cost independent of expression depth.
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
