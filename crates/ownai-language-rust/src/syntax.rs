//! Rust grammar node-kind constants and the parser constructor.
//!
//! Tree-sitter node names are centralized here so that grammar upgrades fail
//! focused tests when node names or shapes change. Every other module in this
//! crate refers to nodes through these constants.

use tree_sitter::{Language, LanguageError, Parser};

/// Tree-sitter node kinds referenced by extraction and rendering.
pub mod node {
    pub const SOURCE_FILE: &str = "source_file";
    pub const ERROR: &str = "ERROR";

    pub const LINE_COMMENT: &str = "line_comment";
    pub const BLOCK_COMMENT: &str = "block_comment";

    pub const ATTRIBUTE_ITEM: &str = "attribute_item";
    pub const INNER_ATTRIBUTE_ITEM: &str = "inner_attribute_item";
    pub const ATTRIBUTE: &str = "attribute";

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
}

/// Named fields accessed through `Node::child_by_field_name`.
pub mod field {
    pub const NAME: &str = "name";
    pub const TYPE: &str = "type";
    pub const BODY: &str = "body";
    pub const TRAIT: &str = "trait";
}

/// The Rust grammar, loaded through the native Tree-sitter runtime.
pub fn language() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}

/// Builds a parser with the Rust grammar assigned. A fresh parser is created
/// per projection operation; see TECHNICAL_DESIGN.md section 9.
pub fn parser() -> Result<Parser, LanguageError> {
    let mut parser = Parser::new();
    parser.set_language(&language())?;
    Ok(parser)
}
