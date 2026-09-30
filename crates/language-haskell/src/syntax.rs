//! Haskell grammar node-kind constants.
//!
//! Tree-sitter node names are centralized here so that grammar upgrades fail
//! focused tests when node names or shapes change.

use base::{ProjectionError, SourceSpan, SupportedPath};
use tree_sitter::{Node, Parser};

// Visible node kinds.
pub const HASKELL: &str = "haskell";
pub const HEADER: &str = "header";
pub const MODULE: &str = "module";
pub const MODULE_ID: &str = "module_id";
pub const EXPORTS: &str = "exports";
pub const IMPORT: &str = "import";
pub const IMPORTS: &str = "imports";
pub const DECLARATIONS: &str = "declarations";
pub const DECLARATION: &str = "declaration";
pub const SIGNATURE: &str = "signature";
pub const FUNCTION: &str = "function";
pub const BIND: &str = "bind";
pub const DATA_TYPE: &str = "data_type";
pub const NEWTYPE: &str = "newtype";
pub const TYPE_SYNONYM: &str = "type_synomym";
pub const CLASS: &str = "class";
pub const INSTANCE: &str = "instance";
pub const DATA_FAMILY: &str = "data_family";
pub const TYPE_FAMILY: &str = "type_family";
pub const TYPE_INSTANCE: &str = "type_instance";
pub const DATA_INSTANCE: &str = "data_instance";
pub const PATTERN_SYNONYM: &str = "pattern_synonym";
pub const DERIVING: &str = "deriving";
pub const DERIVING_INSTANCE: &str = "deriving_instance";
pub const KIND_SIGNATURE: &str = "kind_signature";
pub const TYPE_ROLE: &str = "role_annotation";
pub const FOREIGN_IMPORT: &str = "foreign_import";
pub const FOREIGN_EXPORT: &str = "foreign_export";
pub const PRAGMA: &str = "pragma";
pub const COMMENT: &str = "comment";
pub const HADDOCK: &str = "haddock";
pub const CPP: &str = "cpp";
pub const SPLICE: &str = "splice";
pub const TOP_SPLICE: &str = "top_splice";
pub const QUASIQUOTE: &str = "quasiquote";

pub const DATA_CONSTRUCTORS: &str = "data_constructors";
pub const DATA_CONSTRUCTOR: &str = "data_constructor";
pub const NEWTYPE_CONSTRUCTOR: &str = "newtype_constructor";
pub const FIELDS: &str = "fields";
pub const FIELD: &str = "field";
pub const CLASS_DECLARATIONS: &str = "class_declarations";
pub const INSTANCE_DECLARATIONS: &str = "instance_declarations";
pub const DEFAULT_SIGNATURE: &str = "default_signature";
pub const TYPE_PARAMS: &str = "type_params";
pub const TYPE_PARAM: &str = "type_param";
pub const CONTEXT: &str = "context";
pub const FORALL: &str = "forall";
pub const FIELD_NAME: &str = "field_name";
pub const VARIABLE: &str = "variable";
pub const CONSTRUCTOR: &str = "constructor";
pub const PREFIX_ID: &str = "prefix_id";
pub const INFIX_ID: &str = "infix_id";
pub const INVISIBLE: &str = "invisible";
pub const PATTERN: &str = "pattern";
pub const BINDING_LIST: &str = "binding_list";

// Named-field names used during traversal.
pub const FIELD_NAME_FIELD: &str = "name";
pub const FIELD_MODULE: &str = "module";
pub const FIELD_DECLARATIONS: &str = "declarations";
pub const FIELD_TYPE: &str = "type";
pub const FIELD_BODY: &str = "body";
pub const FIELD_PATTERNS: &str = "patterns";
pub const FIELD_EXPRESSION: &str = "expression";

/// Parses one Haskell source file and rejects any tree containing `ERROR` or
/// missing nodes. `None` parse results and error nodes are fatal so that no
/// caller can emit a partial projection (TECHNICAL_DESIGN.md section 9).
pub(crate) fn parse(
    source: &str,
    path: &SupportedPath,
) -> Result<tree_sitter::Tree, ProjectionError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_haskell::LANGUAGE.into())
        .map_err(|_| ProjectionError::AstInvariant {
            path: path.clone(),
            range: whole_file_span(source),
            detail: "the Haskell grammar could not be assigned to a parser".to_owned(),
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
