//! Swift grammar node-kind constants, parsing, and validation.
//!
//! Tree-sitter node names are centralized here so grammar upgrades fail focused
//! tests when node names or shapes change. Parsing is all-or-nothing: a parse
//! failure, an error/missing node anywhere (including inside a stripped body),
//! an unsupported `_read` accessor, or a malformed conditional sequence is a
//! fatal `ProjectionError`, so no caller can emit a partial projection.
//!
//! The one exception is the pinned grammar's missing production for Swift's
//! empty tuple expression `()`: only its exact empty-parenthesis recovery shape
//! is accepted, and every other error or missing node remains fatal.

use base::{ProjectionError, SourceSpan, SupportedPath};
use tree_sitter::{Node, Parser, Tree};

/// The anonymous token kind Tree-sitter reports for the inserted `!` in its
/// empty-tuple-expression recovery.
const MISSING_UNIT_BANG: &str = "!";

/// Tree-sitter node kinds referenced by extraction and rendering.
pub mod node {
    pub const SOURCE_FILE: &str = "source_file";
    pub const ERROR: &str = "ERROR";

    pub const COMMENT: &str = "comment";
    pub const MULTILINE_COMMENT: &str = "multiline_comment";

    pub const MODIFIERS: &str = "modifiers";
    pub const ATTRIBUTE: &str = "attribute";
    pub const VISIBILITY_MODIFIER: &str = "visibility_modifier";
    pub const INHERITANCE_MODIFIER: &str = "inheritance_modifier";
    pub const PROPERTY_MODIFIER: &str = "property_modifier";
    pub const MEMBER_MODIFIER: &str = "member_modifier";
    pub const PARAMETER_MODIFIER: &str = "parameter_modifier";
    pub const PARAMETER_MODIFIERS: &str = "parameter_modifiers";

    // Declaration families.
    pub const CLASS_DECLARATION: &str = "class_declaration";
    pub const CLASS_BODY: &str = "class_body";
    pub const ENUM_CLASS_BODY: &str = "enum_class_body";
    pub const PROTOCOL_DECLARATION: &str = "protocol_declaration";
    pub const PROTOCOL_BODY: &str = "protocol_body";
    pub const TYPEALIAS_DECLARATION: &str = "typealias_declaration";
    pub const ASSOCIATEDTYPE_DECLARATION: &str = "associatedtype_declaration";
    pub const ENUM_ENTRY: &str = "enum_entry";
    pub const ENUM_TYPE_PARAMETERS: &str = "enum_type_parameters";
    pub const PROPERTY_DECLARATION: &str = "property_declaration";
    pub const COMPUTED_PROPERTY: &str = "computed_property";
    pub const PROTOCOL_PROPERTY_DECLARATION: &str = "protocol_property_declaration";
    pub const PROTOCOL_PROPERTY_REQUIREMENTS: &str = "protocol_property_requirements";
    pub const COMPUTED_GETTER: &str = "computed_getter";
    pub const COMPUTED_SETTER: &str = "computed_setter";
    pub const COMPUTED_MODIFY: &str = "computed_modify";
    pub const GETTER_SPECIFIER: &str = "getter_specifier";
    pub const SETTER_SPECIFIER: &str = "setter_specifier";
    pub const MODIFY_SPECIFIER: &str = "modify_specifier";
    pub const WILLSET_DIDSET_BLOCK: &str = "willset_didset_block";
    pub const WILLSET_CLAUSE: &str = "willset_clause";
    pub const DIDSET_CLAUSE: &str = "didset_clause";
    pub const FUNCTION_DECLARATION: &str = "function_declaration";
    pub const PROTOCOL_FUNCTION_DECLARATION: &str = "protocol_function_declaration";
    pub const INIT_DECLARATION: &str = "init_declaration";
    pub const DEINIT_DECLARATION: &str = "deinit_declaration";
    pub const SUBSCRIPT_DECLARATION: &str = "subscript_declaration";
    pub const OPERATOR_DECLARATION: &str = "operator_declaration";
    pub const MACRO_DECLARATION: &str = "macro_declaration";
    pub const MACRO_DEFINITION: &str = "macro_definition";
    pub const EXTERNAL_MACRO_DEFINITION: &str = "external_macro_definition";
    pub const FUNCTION_BODY: &str = "function_body";

    // Conditional compilation and dropped declarations.
    pub const DIRECTIVE: &str = "directive";
    pub const DIAGNOSTIC: &str = "diagnostic";
    pub const IMPORT_DECLARATION: &str = "import_declaration";
    pub const PRECEDENCE_GROUP_DECLARATION: &str = "precedence_group_declaration";

    // Types.
    pub const USER_TYPE: &str = "user_type";
    pub const TYPE_IDENTIFIER: &str = "type_identifier";
    pub const ARRAY_TYPE: &str = "array_type";
    pub const OPTIONAL_TYPE: &str = "optional_type";
    pub const FUNCTION_TYPE: &str = "function_type";
    pub const TUPLE_TYPE: &str = "tuple_type";
    pub const TUPLE_TYPE_ITEM: &str = "tuple_type_item";
    pub const OPAQUE_TYPE: &str = "opaque_type";
    pub const EXISTENTIAL_TYPE: &str = "existential_type";
    pub const PROTOCOL_COMPOSITION_TYPE: &str = "protocol_composition_type";
    pub const TYPE_PACK_EXPANSION: &str = "type_pack_expansion";
    pub const TYPE_ANNOTATION: &str = "type_annotation";
    pub const TYPE_PARAMETERS: &str = "type_parameters";
    pub const TYPE_PARAMETER: &str = "type_parameter";
    pub const TYPE_CONSTRAINTS: &str = "type_constraints";

    // Expressions involved in the pinned grammar's empty-tuple recovery.
    pub const TUPLE_EXPRESSION: &str = "tuple_expression";
    pub const BANG: &str = "bang";

    // Effects.
    pub const THROWS: &str = "throws";
    pub const THROWS_CLAUSE: &str = "throws_clause";
    pub const MUTATION_MODIFIER: &str = "mutation_modifier";

    // Signatures / attributes / hazards.
    pub const PARAMETER: &str = "parameter";
    pub const PATTERN: &str = "pattern";
    pub const SIMPLE_IDENTIFIER: &str = "simple_identifier";
    pub const CUSTOM_OPERATOR: &str = "custom_operator";
    pub const LAMBDA_LITERAL: &str = "lambda_literal";
    pub const INTERPOLATED_EXPRESSION: &str = "interpolated_expression";
    pub const INHERITANCE_SPECIFIER: &str = "inheritance_specifier";
    pub const VALUE_BINDING_PATTERN: &str = "value_binding_pattern";
    pub const TYPE_ARGUMENTS: &str = "type_arguments";
    pub const STATEMENTS: &str = "statements";
}

/// Named fields accessed through `Node::child_by_field_name`.
pub mod field {
    pub const NAME: &str = "name";
    pub const BODY: &str = "body";
    pub const TYPE: &str = "type";
    pub const DECLARATION_KIND: &str = "declaration_kind";
    pub const DATA_CONTENTS: &str = "data_contents";
    pub const RAW_VALUE: &str = "raw_value";
    pub const MUTABILITY: &str = "mutability";
    pub const DEFAULT_VALUE: &str = "default_value";
    pub const DEFINITION: &str = "definition";
    pub const INHERITS_FROM: &str = "inherits_from";
    pub const WRAPPED: &str = "wrapped";
    pub const VALUE: &str = "value";
    pub const EXTERNAL_NAME: &str = "external_name";
    pub const COMPUTED_VALUE: &str = "computed_value";
}

/// Direct-child node kinds where a declaration's executable body begins. The
/// renderer stops before them and emits a frozen placeholder instead.
pub fn is_body(kind: &str) -> bool {
    matches!(
        kind,
        node::CLASS_BODY
            | node::ENUM_CLASS_BODY
            | node::PROTOCOL_BODY
            | node::FUNCTION_BODY
            | node::COMPUTED_PROPERTY
            | node::PROTOCOL_PROPERTY_REQUIREMENTS
            | node::WILLSET_DIDSET_BLOCK
            | node::MACRO_DEFINITION
            | node::EXTERNAL_MACRO_DEFINITION
            | node::STATEMENTS
            | node::LAMBDA_LITERAL
    )
}

/// Named node kinds that represent a type expression in a signature.
pub fn is_type(kind: &str) -> bool {
    matches!(
        kind,
        node::USER_TYPE
            | node::TYPE_IDENTIFIER
            | node::ARRAY_TYPE
            | node::OPTIONAL_TYPE
            | node::FUNCTION_TYPE
            | node::TUPLE_TYPE
            | node::TUPLE_TYPE_ITEM
            | node::OPAQUE_TYPE
            | node::EXISTENTIAL_TYPE
            | node::PROTOCOL_COMPOSITION_TYPE
            | node::TYPE_PACK_EXPANSION
            | node::TYPE_ANNOTATION
    )
}

pub(crate) fn parse(source: &str, path: &SupportedPath) -> Result<Tree, ProjectionError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .map_err(|_| ProjectionError::AstInvariant {
            path: path.clone(),
            range: whole_file_span(source),
            detail: "the Swift grammar could not be assigned to a parser".to_owned(),
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| ProjectionError::ParseFailed {
            path: path.clone(),
            range: whole_file_span(source),
        })?;

    if tree.root_node().has_error() && !only_empty_tuple_recovery(tree.root_node()) {
        let range = first_error_range(tree.root_node()).unwrap_or_else(|| whole_file_span(source));
        return Err(ProjectionError::ErroneousSyntax {
            path: path.clone(),
            range,
        });
    }

    // A clean misparse: `_read { ... }` alone has no ERROR node but is not a
    // supported accessor. Reject it narrowly (see lib.rs::read_in_accessor_position).
    if let Some((start, end)) = crate::read_in_accessor_position_node(tree.root_node(), source)
        .into_iter()
        .next()
    {
        return Err(ProjectionError::ErroneousSyntax {
            path: path.clone(),
            range: span_for_bytes(source, start, end),
        });
    }

    Ok(tree)
}

/// The pinned grammar has no production for Swift's empty tuple expression `()`.
/// It recovers by inserting a missing `!` under a `bang` node inside a
/// `tuple_expression`, e.g. `(tuple_expression value: (bang (MISSING "!")))`.
/// Accept only that recovery when the tuple itself contains nothing but `(`,
/// the inserted bang, `)`, and comments. Every other ERROR/MISSING node remains
/// fatal, including a missing bang under another parent or a tuple containing a
/// comma, `?`, or any other token.
fn only_empty_tuple_recovery(root: Node<'_>) -> bool {
    let mut stack = vec![root];
    let mut recoveries = 0usize;
    while let Some(node) = stack.pop() {
        if node.is_error() {
            return false;
        }
        if node.is_missing() {
            if !is_empty_tuple_recovery(node) {
                return false;
            }
            recoveries += 1;
        }
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        stack.extend(children);
    }
    recoveries > 0
}

fn is_empty_tuple_recovery(missing: Node<'_>) -> bool {
    if missing.kind() != MISSING_UNIT_BANG {
        return false;
    }
    let Some(bang) = missing.parent() else {
        return false;
    };
    if bang.kind() != node::BANG {
        return false;
    }
    let Some(tuple) = bang.parent() else {
        return false;
    };
    if tuple.kind() != node::TUPLE_EXPRESSION {
        return false;
    }
    let mut cursor = tuple.walk();
    let mut children = tuple
        .children(&mut cursor)
        .filter(|child| !crate::render::is_comment(child.kind()));
    let (Some(open), Some(middle), Some(close)) =
        (children.next(), children.next(), children.next())
    else {
        return false;
    };
    open.kind() == "(" && middle == bang && close.kind() == ")" && children.next().is_none()
}

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

/// Span of a declaration's actual source text, excluding leading attributes
/// and doc comments but including ordinary modifiers such as `public`,
/// `final`, and `static`. Attributes belong to the declaration but precede it
/// in source; the CLI JSON contract reports the declaration itself, so a
/// leading attribute block must not be part of the span. Computed from the
/// parsed tree, never from raw source slicing.
pub(crate) fn declaration_span(node: Node<'_>, source: &str) -> SourceSpan {
    let mut start = node.start_byte();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if crate::render::is_comment(child.kind()) || child.kind() == node::ATTRIBUTE {
                if !cursor.goto_next_sibling() {
                    break;
                }
                continue;
            }
            if child.kind() == node::MODIFIERS {
                let mut modifiers = child.walk();
                let mut found = false;
                for modifier in child.children(&mut modifiers) {
                    if modifier.kind() == node::ATTRIBUTE
                        || crate::render::is_comment(modifier.kind())
                    {
                        continue;
                    }
                    start = modifier.start_byte();
                    found = true;
                    break;
                }
                if found {
                    break;
                }
            } else {
                start = child.start_byte();
                break;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    span_for_bytes(source, start, node.end_byte())
}

pub(crate) fn whole_file_span(source: &str) -> SourceSpan {
    span_for_bytes(source, 0, source.len())
}

/// Build a `SourceSpan` from a byte range in `source`. Tree-sitter byte offsets
/// are always at UTF-8 character boundaries for valid syntax.
pub(crate) fn span_for_bytes(source: &str, start: usize, end: usize) -> SourceSpan {
    let mut line = 0usize;
    let mut column = 0usize;
    let mut start_pos = (0usize, 0usize);
    let mut end_pos = (0usize, 0usize);
    for (offset, character) in source.char_indices() {
        if offset == start {
            start_pos = (line, column);
        }
        if offset == end {
            end_pos = (line, column);
            break;
        }
        if character == '\n' {
            line += 1;
            column = 0;
        } else {
            column += character.len_utf8();
        }
    }
    if start == end {
        end_pos = start_pos;
    } else if end_pos == (0, 0) && end >= source.len() {
        // Walking reached the end of the source without observing `end`.
        end_pos = (line, column);
    }
    SourceSpan::new(start, end, start_pos.0, start_pos.1, end_pos.0, end_pos.1)
}
