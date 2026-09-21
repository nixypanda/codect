//! Canonical rendering of Elm declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs.
//!
//! Structure comes from the parsed tree, never from source slices, so comments
//! and whitespace inside a declaration cannot leak into output. Line breaks are
//! fixed by declaration shape; nothing here consults terminal width.

use ownai_core::{Language, ProjectionError, RepoPath};
use tree_sitter::Node;

use crate::syntax::{
    self, ARROW, DOT, FIELD_ASSOCIATIVITY, FIELD_BASE_RECORD, FIELD_FIELD_TYPE, FIELD_NAME,
    FIELD_OPERATOR, FIELD_PART, FIELD_PRECEDENCE, FIELD_TYPE_EXPRESSION, FIELD_TYPE_NAME,
    FIELD_TYPE_VARIABLE, FIELD_UNION_VARIANT, FIELD_UNIT_EXPR, LOWER_CASE_IDENTIFIER, PORT,
    RECORD_TYPE, TUPLE_TYPE, TYPE_EXPRESSION, TYPE_REF, TYPE_VARIABLE, UPPER_CASE_IDENTIFIER,
    UPPER_CASE_QID, VALUE_EXPR,
};

const INDENT: &str = "    ";

/// A minimal document representation: this is a structural pretty-printer for a
/// handful of declaration shapes, not a general Elm source formatter.
pub(crate) enum Doc {
    Text(String),
    Line,
    Indent(Box<Doc>),
    Concat(Vec<Doc>),
}

impl Doc {
    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, indent: usize) {
        match self {
            Doc::Text(value) => out.push_str(value),
            Doc::Line => {
                out.push('\n');
                for _ in 0..indent {
                    out.push_str(INDENT);
                }
            }
            Doc::Indent(inner) => inner.write(out, indent + 1),
            Doc::Concat(parts) => {
                for part in parts {
                    part.write(out, indent);
                }
            }
        }
    }
}

fn text(value: impl Into<String>) -> Doc {
    Doc::Text(value.into())
}

fn join(parts: Vec<Doc>, separator: &str) -> Doc {
    let mut out = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            out.push(text(separator));
        }
        out.push(part);
    }
    Doc::Concat(out)
}

pub(crate) struct Renderer<'a> {
    path: &'a RepoPath,
    source: &'a str,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(path: &'a RepoPath, source: &'a str) -> Self {
        Self { path, source }
    }

    fn invariant<T>(
        &self,
        node: Node<'_>,
        detail: impl Into<String>,
    ) -> Result<T, ProjectionError> {
        Err(ProjectionError::AstInvariant {
            path: self.path.clone(),
            language: Language::Elm,
            range: syntax::node_span(node),
            detail: detail.into(),
        })
    }

    fn required_field<'n>(&self, node: Node<'n>, field: &str) -> Result<Node<'n>, ProjectionError> {
        match node.child_by_field_name(field) {
            Some(child) => Ok(child),
            None => self.invariant(
                node,
                format!("`{}` is missing its `{field}` field", node.kind()),
            ),
        }
    }

    pub(crate) fn source_text(&self, node: Node<'_>) -> Result<&'a str, ProjectionError> {
        match node.utf8_text(self.source.as_bytes()) {
            Ok(value) => Ok(value),
            Err(_) => self.invariant(node, "node text is not valid UTF-8"),
        }
    }

    /// The declared lower-case or module name. Qualified names are rebuilt from
    /// identifiers and dots so spacing and comments cannot survive.
    pub(crate) fn qualified_name(&self, node: Node<'_>) -> Result<String, ProjectionError> {
        let mut out = String::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                UPPER_CASE_IDENTIFIER | LOWER_CASE_IDENTIFIER => {
                    out.push_str(self.source_text(child)?);
                }
                DOT => out.push('.'),
                other => {
                    return self.invariant(
                        child,
                        format!("unexpected `{other}` inside a qualified name"),
                    );
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn field_name(&self, node: Node<'_>) -> Result<&'a str, ProjectionError> {
        let name = self.required_field(node, FIELD_NAME)?;
        self.source_text(name)
    }

    /// Returns the module's declared name and whether the module is a port
    /// module, which is the only module kind the MVP distinguishes.
    pub(crate) fn module_name_and_port(
        &self,
        node: Node<'_>,
    ) -> Result<(String, bool), ProjectionError> {
        let name = self.qualified_name(self.required_field(node, FIELD_NAME)?)?;
        let mut cursor = node.walk();
        let is_port = node
            .named_children(&mut cursor)
            .any(|child| child.kind() == PORT);
        Ok((name, is_port))
    }

    /// The declared lower-case name at the head of a function declaration.
    pub(crate) fn declaration_name(&self, node: Node<'_>) -> Result<&'a str, ProjectionError> {
        let mut cursor = node.walk();
        match node
            .named_children(&mut cursor)
            .find(|child| child.kind() == LOWER_CASE_IDENTIFIER)
        {
            Some(name) => self.source_text(name),
            None => self.invariant(node, "a function declaration has no declared name"),
        }
    }

    pub(crate) fn operator(&self, node: Node<'_>) -> Result<&'a str, ProjectionError> {
        self.source_text(self.required_field(node, FIELD_OPERATOR)?)
    }

    pub(crate) fn module(&self, node: Node<'_>, is_port: bool) -> Result<Doc, ProjectionError> {
        let name = self.qualified_name(self.required_field(node, FIELD_NAME)?)?;
        let prefix = if is_port { "port module " } else { "module " };
        Ok(text(format!("{prefix}{name}")))
    }

    /// A custom type: `type Name params` followed by one indented line per
    /// constructor, matching the canonical form in TECHNICAL_DESIGN.md 11.2.
    pub(crate) fn type_declaration(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        let name = self.field_name(node)?;
        let parameters = self.parameters(node, FIELD_TYPE_NAME)?;

        let mut members = Vec::new();
        let mut cursor = node.walk();
        let mut first = true;
        for child in node.children_by_field_name(FIELD_UNION_VARIANT, &mut cursor) {
            members.push(Doc::Line);
            members.push(text(if first { "= " } else { "| " }));
            members.push(self.variant_body(child)?);
            first = false;
        }

        Ok(Doc::Concat(vec![
            text(format!("type {name}{parameters}")),
            Doc::Indent(Box::new(Doc::Concat(members))),
        ]))
    }

    pub(crate) fn variant_body(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        let name = self.field_name(node)?;
        let mut parts = vec![text(name)];
        let mut cursor = node.walk();
        for argument in node.children_by_field_name(FIELD_PART, &mut cursor) {
            parts.push(text(" "));
            parts.push(self.type_atom(argument)?);
        }
        Ok(Doc::Concat(parts))
    }

    /// A type alias. A record right-hand side uses the block form from
    /// TECHNICAL_DESIGN.md 11.2; every other alias stays a single line.
    pub(crate) fn type_alias(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        let name = self.field_name(node)?;
        let parameters = self.parameters(node, FIELD_TYPE_VARIABLE)?;
        let right = self.required_field(node, FIELD_TYPE_EXPRESSION)?;
        let header = format!("type alias {name}{parameters} =");

        if let Some(record) = self.sole_record_type(right)
            && self.record_has_fields(record)
        {
            return Ok(Doc::Concat(vec![
                text(header),
                self.record_type(record, true)?,
            ]));
        }

        Ok(Doc::Concat(vec![
            text(format!("{header} ")),
            self.type_expression(right)?,
        ]))
    }

    pub(crate) fn type_annotation(
        &self,
        node: Node<'_>,
        is_port: bool,
    ) -> Result<Doc, ProjectionError> {
        let name = self.field_name(node)?;
        let annotation = self.type_expression(self.required_field(node, FIELD_TYPE_EXPRESSION)?)?;
        let prefix = if is_port { "port " } else { "" };
        Ok(Doc::Concat(vec![
            text(format!("{prefix}{name} : ")),
            annotation,
        ]))
    }

    /// The declared surface for an unannotated top-level declaration. The MVP
    /// never infers a type (PRODUCT.md, "Missing Elm type annotations").
    pub(crate) fn missing_annotation(&self, name: &str) -> Doc {
        text(format!("{name} : <missing type annotation>"))
    }

    pub(crate) fn infix(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        let associativity = self.source_text(self.required_field(node, FIELD_ASSOCIATIVITY)?)?;
        let precedence = self.source_text(self.required_field(node, FIELD_PRECEDENCE)?)?;
        let operator = self.source_text(self.required_field(node, FIELD_OPERATOR)?)?;

        let mut cursor = node.walk();
        let value = match node
            .named_children(&mut cursor)
            .find(|child| child.kind() == VALUE_EXPR)
        {
            Some(value) => value,
            None => {
                return self.invariant(node, "`infix_declaration` is missing its implementation");
            }
        };
        let value_name = self.qualified_name(self.required_field(value, FIELD_NAME)?)?;

        Ok(text(format!(
            "infix {associativity} {precedence} ({operator}) = {value_name}"
        )))
    }

    fn parameters(&self, node: Node<'_>, field: &str) -> Result<String, ProjectionError> {
        let mut out = String::new();
        let mut cursor = node.walk();
        for parameter in node.children_by_field_name(field, &mut cursor) {
            out.push(' ');
            out.push_str(self.source_text(parameter)?);
        }
        Ok(out)
    }

    fn record_has_fields(&self, node: Node<'_>) -> bool {
        let mut cursor = node.walk();
        node.children_by_field_name(FIELD_FIELD_TYPE, &mut cursor)
            .next()
            .is_some()
    }

    /// The grammar wraps a type alias right-hand side in a `type_expression`
    /// even when it is a single record, so a record body is only reachable as
    /// the lone non-arrow atom.
    fn sole_record_type<'n>(&self, node: Node<'n>) -> Option<Node<'n>> {
        if node.kind() != TYPE_EXPRESSION {
            return None;
        }
        let mut cursor = node.walk();
        let mut atoms = node
            .named_children(&mut cursor)
            .filter(|child| child.kind() != ARROW);
        let first = atoms.next()?;
        if atoms.next().is_some() || first.kind() != RECORD_TYPE {
            return None;
        }
        Some(first)
    }

    fn record_type(&self, node: Node<'_>, block: bool) -> Result<Doc, ProjectionError> {
        let base = node.child_by_field_name(FIELD_BASE_RECORD);
        let mut cursor = node.walk();
        let fields: Vec<Node<'_>> = node
            .children_by_field_name(FIELD_FIELD_TYPE, &mut cursor)
            .collect();

        if fields.is_empty() {
            return Ok(match base {
                Some(base) => text(format!("{{ {} | }}", self.source_text(base)?)),
                None => text("{}"),
            });
        }

        let mut rendered = Vec::with_capacity(fields.len());
        for field in &fields {
            rendered.push(self.field_type(*field)?);
        }

        if !block {
            let mut parts = Vec::new();
            if let Some(base) = base {
                parts.push(text(self.source_text(base)?));
                parts.push(text(" | "));
            }
            parts.push(join(rendered, ", "));
            return Ok(Doc::Concat(vec![
                text("{ "),
                Doc::Concat(parts),
                text(" }"),
            ]));
        }

        let first_prefix = match base {
            Some(base) => format!("{{ {} | ", self.source_text(base)?),
            None => "{ ".to_owned(),
        };
        let mut members = Vec::new();
        members.push(Doc::Line);
        members.push(text(first_prefix));
        members.push(rendered.remove(0));
        for field in rendered {
            members.push(Doc::Line);
            members.push(text(", "));
            members.push(field);
        }
        members.push(Doc::Line);
        members.push(text("}"));

        Ok(Doc::Indent(Box::new(Doc::Concat(members))))
    }

    pub(crate) fn field_type(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        let name = self.field_name(node)?;
        let field_type = self.type_expression(self.required_field(node, FIELD_TYPE_EXPRESSION)?)?;
        Ok(Doc::Concat(vec![text(format!("{name} : ")), field_type]))
    }

    fn type_expression(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        if node.kind() != TYPE_EXPRESSION {
            return self.type_atom(node);
        }

        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == ARROW {
                continue;
            }
            parts.push(self.type_atom(child)?);
        }
        Ok(join(parts, " -> "))
    }

    fn type_atom(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        match node.kind() {
            TYPE_REF => self.type_ref(node),
            TYPE_VARIABLE => Ok(text(self.source_text(node)?)),
            RECORD_TYPE => self.record_type(node, false),
            TUPLE_TYPE => self.tuple_type(node),
            // A parenthesized type expression only exists as an atom because
            // the wrapping `(` `)` tokens belong to a hidden grammar rule.
            TYPE_EXPRESSION => Ok(Doc::Concat(vec![
                text("("),
                self.type_expression(node)?,
                text(")"),
            ])),
            other => self.invariant(node, format!("unexpected `{other}` in a type expression")),
        }
    }

    fn type_ref(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == UPPER_CASE_QID {
                parts.push(text(self.qualified_name(child)?));
            } else {
                parts.push(text(" "));
                parts.push(self.type_atom(child)?);
            }
        }
        Ok(Doc::Concat(parts))
    }

    fn tuple_type(&self, node: Node<'_>) -> Result<Doc, ProjectionError> {
        if node.child_by_field_name(FIELD_UNIT_EXPR).is_some() {
            return Ok(text("()"));
        }

        let mut members = Vec::new();
        let mut cursor = node.walk();
        for member in node.children_by_field_name(FIELD_TYPE_EXPRESSION, &mut cursor) {
            members.push(self.type_expression(member)?);
        }
        Ok(Doc::Concat(vec![text("("), join(members, ", "), text(")")]))
    }
}
