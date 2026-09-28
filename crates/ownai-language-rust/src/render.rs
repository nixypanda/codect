//! Canonical rendering of Rust declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs (TECHNICAL_DESIGN.md
//! sections 10 and 12). Node text is never copied verbatim: declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks. Terminal width is not consulted anywhere in this module.

use ownai_core::LINE_WIDTH;
use tree_sitter::Node;
use unicode_width::UnicodeWidthStr;

use crate::syntax::{field, node};

/// A small layout document. `Indent` is relative, so the same document can be
/// rendered at any nesting depth and a nested item's own fragment matches the
/// text embedded in its ancestor.
///
/// [`Doc::Group`] is the only width-sensitive construct: it renders flat (soft
/// breaks as spaces) when the flat form fits within [`LINE_WIDTH`], and broken
/// (soft breaks as newlines) otherwise.
#[derive(Clone, Debug)]
pub(crate) enum Doc {
    Text(String),
    Line,
    /// A space when flat, a line break when broken.
    SoftLine,
    /// Nothing when flat, a line break when broken.
    SoftNil,
    /// Text emitted only when the enclosing group is broken, used for the
    /// trailing comma that keeps an appended list item on a single diff line.
    Broken(&'static str),
    Indent(Box<Doc>),
    Group(Box<Doc>),
    Concat(Vec<Doc>),
}

struct Output {
    text: String,
    depth: usize,
    at_line_start: bool,
    column: usize,
}

impl Output {
    fn write(&mut self, value: &str) {
        if self.at_line_start {
            for _ in 0..self.depth {
                self.text.push_str("    ");
            }
            self.column = self.depth * 4;
            self.at_line_start = false;
        }
        self.text.push_str(value);
        self.column += UnicodeWidthStr::width(value);
    }

    fn line(&mut self) {
        self.text.push('\n');
        self.at_line_start = true;
        self.column = 0;
    }

    /// The column the next written character would land in.
    fn column_now(&self) -> usize {
        if self.at_line_start {
            self.depth * 4
        } else {
            self.column
        }
    }
}

/// Renders a document at a nesting depth of four spaces per level.
pub(crate) fn render(doc: &Doc, depth: usize) -> String {
    let mut output = Output {
        text: String::new(),
        depth,
        at_line_start: true,
        column: 0,
    };
    render_into(doc, &mut output, false);
    output.text
}

fn render_into(doc: &Doc, output: &mut Output, flat: bool) {
    match doc {
        Doc::Text(value) => output.write(value),
        Doc::Line => output.line(),
        Doc::SoftLine => {
            if flat {
                output.write(" ");
            } else {
                output.line();
            }
        }
        Doc::SoftNil => {
            if !flat {
                output.line();
            }
        }
        Doc::Broken(value) => {
            if !flat {
                output.write(value);
            }
        }
        Doc::Indent(inner) => {
            output.depth += 1;
            render_into(inner, output, flat);
            output.depth -= 1;
        }
        Doc::Group(inner) => {
            let flat_here = flat
                || flat_width(inner).is_some_and(|width| output.column_now() + width <= LINE_WIDTH);
            render_into(inner, output, flat_here);
        }
        Doc::Concat(parts) => {
            for part in parts {
                render_into(part, output, flat);
            }
        }
    }
}

/// The display width of `doc` rendered flat, or `None` when it contains a hard
/// [`Doc::Line`] and can therefore never be flat.
fn flat_width(doc: &Doc) -> Option<usize> {
    match doc {
        Doc::Text(value) => Some(UnicodeWidthStr::width(value.as_str())),
        Doc::Line => None,
        Doc::SoftLine => Some(1),
        Doc::SoftNil | Doc::Broken(_) => Some(0),
        Doc::Indent(inner) | Doc::Group(inner) => flat_width(inner),
        Doc::Concat(parts) => {
            let mut total = 0;
            for part in parts {
                total += flat_width(part)?;
            }
            Some(total)
        }
    }
}

fn is_comment(kind: &str) -> bool {
    matches!(kind, node::LINE_COMMENT | node::BLOCK_COMMENT)
}

/// Literals whose internal bytes must be preserved exactly, including spaces
/// inside string contents and characters.
fn is_atomic_literal(kind: &str) -> bool {
    matches!(
        kind,
        node::STRING_LITERAL
            | node::RAW_STRING_LITERAL
            | node::CHAR_LITERAL
            | node::INTEGER_LITERAL
            | node::FLOAT_LITERAL
            | node::BOOLEAN_LITERAL
            | node::NEGATIVE_LITERAL
    )
}

fn push_tokens(node: Node<'_>, source: &str, out: &mut Vec<String>) {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        let kind = current.kind();
        if is_comment(kind) {
            continue;
        }
        if current.child_count() == 0 || is_atomic_literal(kind) {
            if let Some(text) = source.get(current.byte_range()) {
                out.push(text.to_owned());
            }
            continue;
        }
        let mut children: Vec<Node<'_>> = Vec::new();
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            children.push(child);
        }
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
}

pub(crate) fn tokens(node: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    push_tokens(node, source, &mut out);
    out
}

/// Spacing is a function of the two adjacent leaf tokens, never of source
/// whitespace. Optional trailing commas before a closing angle bracket or at
/// the end of a fragment are dropped, because they carry no meaning.
pub(crate) fn join(tokens: &[String]) -> String {
    let mut text = String::new();
    let mut previous: Option<&str> = None;
    for (index, token) in tokens.iter().enumerate() {
        if token == "," && tokens.get(index + 1).is_none_or(|next| next == ">") {
            continue;
        }
        if let Some(previous) = previous
            && needs_space(previous, token)
        {
            text.push(' ');
        }
        text.push_str(token);
        previous = Some(token);
    }
    text
}

fn needs_space(previous: &str, next: &str) -> bool {
    if previous.is_empty() || next.is_empty() {
        return false;
    }

    // Punctuation that always attaches to the preceding token.
    if matches!(next, ")" | "]" | "," | ";" | ":" | "::" | "." | ">") {
        return false;
    }

    // Opening delimiters normally attach, except after a separator or operator.
    if matches!(next, "<" | "(" | "[") {
        return matches!(previous, "," | ";" | ":" | "=" | "->" | "+" | "=>");
    }

    // A never-type or macro bang is written apart from what precedes it.
    if next == "!" {
        return true;
    }

    // Opening delimiters and prefix punctuation attach to what follows them.
    if matches!(
        previous,
        "(" | "[" | "<" | "&" | "*" | "::" | "." | "#" | "?" | "'" | "!" | "-"
    ) {
        return false;
    }

    // Separators and infix operators are surrounded by one space.
    if matches!(previous, "," | ";" | ":" | "=" | "->" | "+" | "=>") {
        return true;
    }
    if matches!(next, "=" | "->" | "+") {
        return true;
    }

    // A closing angle bracket ends a path or argument list.
    if previous == ">" {
        return true;
    }

    true
}

pub(crate) fn render_node(node: Node<'_>, source: &str) -> String {
    join(&tokens(node, source))
}

fn child_by_field_name<'t>(node: Node<'t>, name: &str) -> Option<Node<'t>> {
    node.child_by_field_name(name)
}

pub(crate) fn child_of_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

/// One element of a declaration header: either flattened tokens or a bracketed
/// list that is allowed to wrap.
enum Elem {
    Token(String),
    List {
        items: Vec<String>,
        open: &'static str,
        close: &'static str,
    },
}

/// Renders the declaration prefix up to the first child in `stop_kinds`,
/// dropping statement terminators that the caller re-emits.
///
/// Parameter and type-parameter lists become groups that wrap at [`LINE_WIDTH`];
/// every other token keeps the fixed spacing rules, so a header that fits is
/// byte-for-byte the old flat output.
pub(crate) fn header(node: Node<'_>, source: &str, stop_kinds: &[&str]) -> Doc {
    let elements = header_elements(node, source, stop_kinds);
    elements_doc(&elements)
}

fn header_elements(node: Node<'_>, source: &str, stop_kinds: &[&str]) -> Vec<Elem> {
    let mut elements = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if stop_kinds.contains(&kind) {
            break;
        }
        if kind == node::SEMICOLON {
            continue;
        }
        if kind == node::PARAMETERS {
            elements.push(list_elem(child, source, "(", ")"));
        } else if kind == node::TYPE_PARAMETERS {
            elements.push(list_elem(child, source, "<", ">"));
        } else {
            for token in tokens(child, source) {
                elements.push(Elem::Token(token));
            }
        }
    }
    elements
}

fn list_elem(node: Node<'_>, source: &str, open: &'static str, close: &'static str) -> Elem {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        items.push(render_node(child, source));
    }
    Elem::List { items, open, close }
}

fn elements_doc(elements: &[Elem]) -> Doc {
    let mut parts: Vec<Doc> = Vec::new();
    let mut previous: Option<&str> = None;
    for (index, element) in elements.iter().enumerate() {
        match element {
            Elem::Token(token) => {
                if token == "," {
                    let dropped = match elements.get(index + 1) {
                        None => true,
                        Some(Elem::Token(next)) => next == ">",
                        Some(Elem::List { open, .. }) => *open == ">",
                    };
                    if dropped {
                        continue;
                    }
                }
                if let Some(previous) = previous
                    && needs_space(previous, token)
                {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                parts.push(Doc::Text(token.clone()));
                previous = Some(token);
            }
            Elem::List { items, open, close } => {
                if let Some(previous) = previous
                    && needs_space(previous, open)
                {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                parts.push(bracket_list_doc(items, open, close));
                previous = Some(close);
            }
        }
    }
    Doc::Concat(parts)
}

/// A bracketed list rendered inline when it fits [`LINE_WIDTH`] and one item per
/// indented line otherwise. The trailing comma is emitted only when broken, so
/// appending an item changes exactly one line.
fn bracket_list_doc(items: &[String], open: &str, close: &str) -> Doc {
    if items.is_empty() {
        return Doc::Text(format!("{open}{close}"));
    }
    let mut inner: Vec<Doc> = vec![Doc::SoftNil];
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            inner.push(Doc::Text(",".to_owned()));
            inner.push(Doc::SoftLine);
        }
        inner.push(Doc::Text(item.clone()));
    }
    inner.push(Doc::Broken(","));
    Doc::Group(Box::new(Doc::Concat(vec![
        Doc::Text(open.to_owned()),
        Doc::Indent(Box::new(Doc::Concat(inner))),
        Doc::SoftNil,
        Doc::Text(close.to_owned()),
    ])))
}

pub(crate) fn where_clause_text(node: Node<'_>, source: &str) -> Option<String> {
    child_of_kind(node, node::WHERE_CLAUSE).map(|clause| render_node(clause, source))
}

pub(crate) fn attribute_text(attribute: Node<'_>, source: &str) -> String {
    render_node(attribute, source)
}

/// A unit or tuple struct terminator, or any declaration that ends in `;`.
pub(crate) fn signature_doc(header: Doc, where_clause: Option<String>) -> Doc {
    match where_clause {
        None => Doc::Concat(vec![header, Doc::Text(";".to_owned())]),
        Some(clause) => Doc::Concat(vec![
            header,
            Doc::Line,
            Doc::Text(clause),
            Doc::Text(";".to_owned()),
        ]),
    }
}

/// A brace-delimited container. A `where` clause pushes the opening brace onto
/// its own line, matching the canonical examples in section 12.2.
pub(crate) fn container_doc(header: Doc, where_clause: Option<String>, members: Vec<Doc>) -> Doc {
    if members.is_empty() {
        return match where_clause {
            None => Doc::Concat(vec![header, Doc::Text(" {}".to_owned())]),
            Some(clause) => Doc::Concat(vec![
                header,
                Doc::Line,
                Doc::Text(clause),
                Doc::Line,
                Doc::Text("{}".to_owned()),
            ]),
        };
    }

    let mut body = Vec::new();
    for (index, member) in members.into_iter().enumerate() {
        if index > 0 {
            body.push(Doc::Line);
        }
        body.push(member);
    }

    let open = match where_clause {
        None => vec![header, Doc::Text(" {".to_owned()), Doc::Line],
        Some(clause) => vec![
            header,
            Doc::Line,
            Doc::Text(clause),
            Doc::Line,
            Doc::Text("{".to_owned()),
            Doc::Line,
        ],
    };

    let mut parts = open;
    parts.push(Doc::Indent(Box::new(Doc::Concat(body))));
    parts.push(Doc::Line);
    parts.push(Doc::Text("}".to_owned()));
    Doc::Concat(parts)
}

/// Prefixes a declaration with its preserved outer attributes, one per line.
pub(crate) fn with_attributes(attributes: Vec<Doc>, declaration: Doc) -> Doc {
    if attributes.is_empty() {
        return declaration;
    }
    let mut parts = Vec::new();
    for attribute in attributes {
        parts.push(attribute);
        parts.push(Doc::Line);
    }
    parts.push(declaration);
    Doc::Concat(parts)
}

fn field_texts(list: Node<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == node::FIELD_DECLARATION {
            out.push(render_node(child, source));
        }
    }
    out
}

pub(crate) fn variant_text(variant: Node<'_>, source: &str) -> String {
    let name = child_by_field_name(variant, field::NAME)
        .map(|node| render_node(node, source))
        .unwrap_or_default();

    match child_by_field_name(variant, field::BODY) {
        None => render_node(variant, source),
        Some(body) if body.kind() == node::ORDERED_FIELD_DECLARATION_LIST => {
            format!("{name}{}", render_node(body, source))
        }
        Some(body) => {
            let fields = field_texts(body, source);
            if fields.is_empty() {
                format!("{name} {{}}")
            } else {
                format!("{name} {{ {} }}", fields.join(", "))
            }
        }
    }
}

pub(crate) fn field_text(field: Node<'_>, source: &str) -> String {
    render_node(field, source)
}

/// `type Item = RightHandSide`, without the terminator or where clause.
pub(crate) fn type_alias_text(node: Node<'_>, source: &str) -> String {
    let mut out = Vec::new();
    if let Some(visibility) = child_of_kind(node, node::VISIBILITY_MODIFIER) {
        push_tokens(visibility, source, &mut out);
    }
    out.push("type".to_owned());
    if let Some(name) = child_by_field_name(node, field::NAME) {
        push_tokens(name, source, &mut out);
    }
    if let Some(parameters) = child_of_kind(node, node::TYPE_PARAMETERS) {
        push_tokens(parameters, source, &mut out);
    }
    out.push(node::EQUAL.to_owned());
    if let Some(right) = child_by_field_name(node, field::TYPE) {
        push_tokens(right, source, &mut out);
    }
    join(&out)
}

/// `type Error: Bound + Bound`, without the terminator or where clause.
pub(crate) fn associated_type_text(node: Node<'_>, source: &str) -> String {
    let mut out = vec!["type".to_owned()];
    if let Some(name) = child_by_field_name(node, field::NAME) {
        push_tokens(name, source, &mut out);
    }
    if let Some(parameters) = child_of_kind(node, node::TYPE_PARAMETERS) {
        push_tokens(parameters, source, &mut out);
    }
    if let Some(bounds) = child_of_kind(node, node::TRAIT_BOUNDS) {
        push_tokens(bounds, source, &mut out);
    }
    join(&out)
}

/// A constant or static declaration with its initializer removed.
pub(crate) fn constant_text(node: Node<'_>, source: &str) -> String {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == node::EQUAL {
            break;
        }
        if kind == node::SEMICOLON {
            continue;
        }
        push_tokens(child, source, &mut out);
    }
    join(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spaced(parts: &[&str]) -> String {
        let tokens: Vec<String> = parts.iter().map(|part| (*part).to_owned()).collect();
        join(&tokens)
    }

    #[test]
    fn spacing_normalizes_type_and_signature_tokens() {
        assert_eq!(
            spaced(&["pub", "struct", "User", "<", "T", ">"]),
            "pub struct User<T>"
        );
        assert_eq!(spaced(&["T", ":", "Send", "+", "Sync"]), "T: Send + Sync");
        assert_eq!(spaced(&["&", "mut", "T"]), "&mut T");
        assert_eq!(spaced(&["&", "'", "a", "str"]), "&'a str");
        assert_eq!(spaced(&["[", "U", ";", "N", "]"]), "[U; N]");
        assert_eq!(spaced(&["(", "a", ",", "b", ")"]), "(a, b)");
        assert_eq!(
            spaced(&["Point", "{", "x", ",", "y", "}"]),
            "Point { x, y }"
        );
        assert_eq!(spaced(&["Fn", "(", ")", "->", "i32"]), "Fn() -> i32");
        assert_eq!(spaced(&["T", ":", "?", "Sized"]), "T: ?Sized");
        assert_eq!(spaced(&["impl", "!", "Send"]), "impl !Send");
        assert_eq!(
            spaced(&["std", "::", "error", "::", "Error"]),
            "std::error::Error"
        );
        assert_eq!(
            spaced(&["type", "X", "=", "(", "A", ",", "B", ")"]),
            "type X = (A, B)"
        );
    }

    #[test]
    fn optional_trailing_commas_are_dropped() {
        assert_eq!(spaced(&["Foo", "<", "T", ",", ">"]), "Foo<T>");
        assert_eq!(spaced(&["where", "T", ":", "Clone", ","]), "where T: Clone");
    }

    #[test]
    fn bracket_list_stays_inline_when_it_fits() {
        let items = vec!["a: i32".to_owned(), "b: i32".to_owned()];
        assert_eq!(
            render(&bracket_list_doc(&items, "(", ")"), 0),
            "(a: i32, b: i32)"
        );
    }

    #[test]
    fn bracket_list_wraps_over_the_budget_with_a_trailing_comma() {
        let first = "x".repeat(40);
        let second = "y".repeat(40);
        let items = vec![first.clone(), second.clone()];
        assert_eq!(
            render(&bracket_list_doc(&items, "(", ")"), 0),
            format!("(\n    {first},\n    {second},\n)")
        );
    }
}
