//! Canonical rendering of Rust declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs (TECHNICAL_DESIGN.md
//! sections 10 and 12). Node text is never copied verbatim: declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks. Terminal width is not consulted anywhere in this module.

pub(crate) use base::doc::{Doc, render};
use tree_sitter::Node;

use crate::syntax::{field, node};

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

/// One element of a declaration header: either flattened tokens, a recursively
/// rendered type, or a bracketed list that is allowed to wrap.
enum Elem {
    Token(String),
    Type {
        doc: Doc,
        first: String,
        last: String,
    },
    List {
        items: Vec<Doc>,
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
/// The declaration prefix as an ungrouped document. The caller groups it
/// together with whatever shares its first line, so the fit check counts the
/// terminator or opening brace too.
pub(crate) fn header(node: Node<'_>, source: &str, stop_kinds: &[&str]) -> Doc {
    let elements = header_elements(node, source, stop_kinds);
    elements_doc(&elements)
}

fn header_elements(node: Node<'_>, source: &str, stop_kinds: &[&str]) -> Vec<Elem> {
    let mut elements = Vec::new();
    let return_type = node.child_by_field_name(field::RETURN_TYPE);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if stop_kinds.contains(&kind) {
            break;
        }
        if kind == node::SEMICOLON {
            continue;
        }
        if Some(child.id()) == return_type.map(|node| node.id()) {
            let leaves = tokens(child, source);
            if let (Some(first), Some(last)) = (leaves.first(), leaves.last()) {
                elements.push(Elem::Type {
                    doc: type_doc(child, source),
                    first: first.clone(),
                    last: last.clone(),
                });
                continue;
            }
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
        if child.kind() == node::PARAMETER
            && child.child_by_field_name(field::PATTERN).is_some()
            && child.child_by_field_name(field::TYPE).is_some()
        {
            items.push(parameter_doc(child, source));
        } else {
            items.push(Doc::Text(render_node(child, source)));
        }
    }
    Elem::List { items, open, close }
}

/// A named parameter's document: its pattern, `: `, and its recursively
/// rendered type. A parameter type can therefore break inside an already-broken
/// parameter list. Anything else (notably `self`) is rendered flat.
fn parameter_doc(node: Node<'_>, source: &str) -> Doc {
    match child_by_field_name(node, field::TYPE) {
        Some(ty) => prefix_then_type(node, source, ty),
        None => Doc::Text(render_node(node, source)),
    }
}

/// A recursive type document. Only the node kinds below are rebuilt; every
/// other kind falls back to the flat token rendering, so an unrecognized type
/// cannot change its flat output. Each bracket list is grouped so it re-decides
/// its own fit after an enclosing list breaks.
pub(crate) fn type_doc(node: Node<'_>, source: &str) -> Doc {
    match node.kind() {
        node::GENERIC_TYPE => {
            let mut parts = Vec::new();
            if let Some(ty) = child_by_field_name(node, field::TYPE) {
                parts.push(type_doc(ty, source));
            }
            if let Some(arguments) = child_by_field_name(node, field::TYPE_ARGUMENTS) {
                let items = named_type_children(arguments, source);
                parts.push(bracket_list(&items, "<", ">"));
            }
            Doc::Group(Box::new(Doc::Concat(parts)))
        }
        node::TUPLE_TYPE => {
            let items = named_type_children(node, source);
            Doc::Group(Box::new(bracket_list(&items, "(", ")")))
        }
        node::REFERENCE_TYPE => {
            let Some(inner) = child_by_field_name(node, field::TYPE) else {
                return Doc::Text(render_node(node, source));
            };
            let mut prefix_tokens = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.id() == inner.id() {
                    break;
                }
                push_tokens(child, source, &mut prefix_tokens);
            }
            let mut parts = vec![Doc::Text(join(&prefix_tokens))];
            let first = tokens(inner, source).into_iter().next().unwrap_or_default();
            if needs_space(last_or_empty(&prefix_tokens), &first) {
                parts.push(Doc::Text(" ".to_owned()));
            }
            parts.push(type_doc(inner, source));
            Doc::Concat(parts)
        }
        node::FUNCTION_TYPE => {
            let parameters = child_by_field_name(node, field::PARAMETERS);
            let mut parts = Vec::new();
            // The prefix before `parameters` is the `fn` keyword for a bare
            // function type, or the trait name (`Fn`) when nested under
            // `impl`/`dyn`.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if Some(child.id()) == parameters.map(|node| node.id()) {
                    break;
                }
                let mut prefix = Vec::new();
                push_tokens(child, source, &mut prefix);
                parts.push(Doc::Text(join(&prefix)));
            }
            if let Some(parameters) = parameters {
                let items = named_type_children(parameters, source);
                parts.push(bracket_list(&items, "(", ")"));
            }
            if let Some(return_type) = child_by_field_name(node, field::RETURN_TYPE) {
                parts.push(Doc::Text(" -> ".to_owned()));
                parts.push(type_doc(return_type, source));
            }
            Doc::Group(Box::new(Doc::Concat(parts)))
        }
        _ => Doc::Text(render_node(node, source)),
    }
}

fn named_type_children(node: Node<'_>, source: &str) -> Vec<Doc> {
    let mut items = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        items.push(type_doc(child, source));
    }
    items
}

fn last_or_empty(tokens: &[String]) -> &str {
    tokens.last().map(String::as_str).unwrap_or("")
}

fn elements_doc(elements: &[Elem]) -> Doc {
    // The last list is the primary break point: it stays ungrouped so the
    // enclosing header group's fit check includes any trailing return type.
    // Earlier lists (generics before parameters) stay grouped so they re-decide
    // and remain inline when they fit on their own.
    let primary = elements
        .iter()
        .rposition(|element| matches!(element, Elem::List { .. }));
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
                        Some(Elem::Type { .. }) => false,
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
                let list = bracket_list(items, open, close);
                if Some(index) == primary {
                    parts.push(list);
                } else {
                    parts.push(Doc::Group(Box::new(list)));
                }
                previous = Some(close);
            }
            Elem::Type { doc, first, last } => {
                if let Some(previous) = previous
                    && needs_space(previous, first)
                {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                parts.push(doc.clone());
                previous = Some(last);
            }
        }
    }
    Doc::Concat(parts)
}

/// A bracketed list body: inline while the enclosing [`Doc::Group`] fits, one
/// item per indented line when it breaks. The trailing comma is emitted only
/// when broken, so appending an item changes exactly one line. This is
/// deliberately ungrouped so the caller can group the whole header.
fn bracket_list(items: &[Doc], open: &str, close: &str) -> Doc {
    if items.is_empty() {
        return Doc::Text(format!("{open}{close}"));
    }
    let mut inner: Vec<Doc> = vec![Doc::SoftNil];
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            inner.push(Doc::Text(",".to_owned()));
            inner.push(Doc::SoftLine);
        }
        inner.push(item.clone());
    }
    inner.push(Doc::Broken(","));
    Doc::Concat(vec![
        Doc::Text(open.to_owned()),
        Doc::Indent(Box::new(Doc::Concat(inner))),
        Doc::SoftNil,
        Doc::Text(close.to_owned()),
    ])
}

pub(crate) fn where_clause_text(node: Node<'_>, source: &str) -> Option<String> {
    child_of_kind(node, node::WHERE_CLAUSE).map(|clause| render_node(clause, source))
}

/// The depth-1 comma-separated argument items of an attribute's `token_tree`,
/// or `None` when there is no argument list or it has no top-level comma.
///
/// The `token_tree` includes its own outer delimiters as direct children; they
/// are dropped so the caller can re-emit the list through [`bracket_list`].
/// Nested token trees are single children, so a comma inside one never splits.
fn attribute_argument_items(attribute: Node<'_>, source: &str) -> Option<Vec<String>> {
    let arguments = attribute.child_by_field_name(field::ARGUMENTS)?;
    if arguments.kind() != node::TOKEN_TREE {
        return None;
    }
    let mut cursor = arguments.walk();
    let children: Vec<Node<'_>> = arguments.children(&mut cursor).collect();
    let inner: &[Node<'_>] = match (children.first(), children.last()) {
        (Some(first), Some(last))
            if matches!(
                (first.kind(), last.kind()),
                (node::OPEN_PAREN, node::CLOSE_PAREN)
                    | (node::OPEN_BRACKET, node::CLOSE_BRACKET)
                    | (node::OPEN_BRACE, node::CLOSE_BRACE)
            ) =>
        {
            &children[1..children.len() - 1]
        }
        _ => &children,
    };

    let mut items = Vec::new();
    let mut segment = Vec::new();
    let mut depth: usize = 0;
    let mut saw_comma = false;
    for child in inner {
        let kind = child.kind();
        if depth == 0 && kind == "," {
            saw_comma = true;
            items.push(join(&segment));
            segment.clear();
            continue;
        }
        match kind {
            node::OPEN_PAREN | node::OPEN_BRACKET | node::OPEN_BRACE => depth += 1,
            node::CLOSE_PAREN | node::CLOSE_BRACKET | node::CLOSE_BRACE => {
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        push_tokens(*child, source, &mut segment);
    }
    if !saw_comma {
        return None;
    }
    items.push(join(&segment));
    Some(items)
}

/// An attribute as a document. A multi-argument attribute whose flat form does
/// not fit wraps one argument per indented line with a trailing comma; anything
/// else is the flat token rendering, byte-identical to before. The `#[` attaches
/// to the path and `]` to the closing `)` through literal text, never
/// [`needs_space`].
pub(crate) fn attribute_doc(attribute_item: Node<'_>, source: &str) -> Doc {
    // Extraction passes the `attribute_item`, whose only named child is the
    // `attribute` carrying the path and the argument token tree.
    let attribute = if attribute_item.kind() == node::ATTRIBUTE {
        attribute_item
    } else {
        match child_of_kind(attribute_item, node::ATTRIBUTE) {
            Some(attribute) => attribute,
            None => return Doc::Text(render_node(attribute_item, source)),
        }
    };
    let arguments = attribute.child_by_field_name(field::ARGUMENTS);
    let mut cursor = attribute.walk();
    let path = attribute
        .named_children(&mut cursor)
        .find(|child| Some(*child) != arguments);
    let (Some(path), Some(items)) = (path, attribute_argument_items(attribute, source)) else {
        return Doc::Text(render_node(attribute_item, source));
    };
    let arguments: Vec<Doc> = items.into_iter().map(Doc::Text).collect();
    Doc::Group(Box::new(Doc::Concat(vec![
        Doc::Text("#[".to_owned()),
        Doc::Text(render_node(path, source)),
        bracket_list(&arguments, "(", ")"),
        Doc::Text("]".to_owned()),
    ])))
}

/// A unit or tuple struct terminator, or any declaration that ends in `;`.
///
/// Without a `where` clause the header shares its line with the `;`, so both
/// form one group. With a clause the header sits alone and the `;` follows the
/// clause on its own line.
pub(crate) fn signature_doc(header: Doc, where_clause: Option<String>) -> Doc {
    match where_clause {
        None => Doc::Group(Box::new(Doc::Concat(vec![
            header,
            Doc::Text(";".to_owned()),
        ]))),
        Some(clause) => Doc::Concat(vec![
            Doc::Group(Box::new(header)),
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
            None => Doc::Group(Box::new(Doc::Concat(vec![
                header,
                Doc::Text(" {}".to_owned()),
            ]))),
            Some(clause) => Doc::Concat(vec![
                Doc::Group(Box::new(header)),
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
        None => vec![
            Doc::Group(Box::new(Doc::Concat(vec![
                header,
                Doc::Text(" {".to_owned()),
            ]))),
            Doc::Line,
        ],
        Some(clause) => vec![
            Doc::Group(Box::new(header)),
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

fn field_docs(list: Node<'_>, source: &str) -> Vec<Doc> {
    let mut out = Vec::new();
    let mut cursor = list.walk();
    for child in list.children(&mut cursor) {
        if child.kind() == node::FIELD_DECLARATION {
            out.push(field_doc(child, source));
        }
    }
    out
}

/// A named field's document: everything before the declared type (visibility,
/// name, and `:`), then the recursively rendered type. The flat rendering is
/// byte-identical to the previous `render_node(field)`, but a long type can now
/// break.
pub(crate) fn field_doc(node: Node<'_>, source: &str) -> Doc {
    match child_by_field_name(node, field::TYPE) {
        Some(ty) => prefix_then_type(node, source, ty),
        None => Doc::Text(render_node(node, source)),
    }
}

/// Everything before `ty` (its sibling tokens are rendered with [`join`]), then
/// a space when required, then the recursively rendered type. This keeps the
/// prefix byte-identical to [`render_node`] while letting `ty` break.
fn prefix_then_type(node: Node<'_>, source: &str, ty: Node<'_>) -> Doc {
    let mut prefix = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.id() == ty.id() {
            break;
        }
        push_tokens(child, source, &mut prefix);
    }
    let mut parts = vec![Doc::Text(join(&prefix))];
    let first = tokens(ty, source).into_iter().next().unwrap_or_default();
    if needs_space(last_or_empty(&prefix), &first) {
        parts.push(Doc::Text(" ".to_owned()));
    }
    parts.push(type_doc(ty, source));
    Doc::Concat(parts)
}

/// A variant body as an ungrouped document. A struct-like variant's field list
/// is a brace list the caller can group with the trailing comma.
pub(crate) fn variant_doc(variant: Node<'_>, source: &str) -> Doc {
    let name = child_by_field_name(variant, field::NAME)
        .map(|node| render_node(node, source))
        .unwrap_or_default();

    match child_by_field_name(variant, field::BODY) {
        None => Doc::Text(render_node(variant, source)),
        Some(body) if body.kind() == node::ORDERED_FIELD_DECLARATION_LIST => {
            Doc::Text(format!("{name}{}", render_node(body, source)))
        }
        Some(body) => {
            let fields = field_docs(body, source);
            if fields.is_empty() {
                Doc::Text(format!("{name} {{}}"))
            } else {
                Doc::Concat(vec![Doc::Text(name), brace_list(&fields)])
            }
        }
    }
}

/// A brace-delimited field list, spaced inline (`{ a: A, b: B }`) and one field
/// per indented line when broken.
fn brace_list(fields: &[Doc]) -> Doc {
    let mut inner: Vec<Doc> = vec![Doc::SoftLine];
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            inner.push(Doc::Text(",".to_owned()));
            inner.push(Doc::SoftLine);
        }
        inner.push(field.clone());
    }
    inner.push(Doc::Broken(","));
    Doc::Concat(vec![
        Doc::Text(" {".to_owned()),
        Doc::Indent(Box::new(Doc::Concat(inner))),
        Doc::SoftLine,
        Doc::Text("}".to_owned()),
    ])
}

/// `type Item = RightHandSide`, without the terminator or where clause.
pub(crate) fn type_alias_text(node: Node<'_>, source: &str) -> Doc {
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
    let mut parts = vec![Doc::Text(join(&out))];
    if let Some(right) = child_by_field_name(node, field::TYPE) {
        let first = tokens(right, source).into_iter().next().unwrap_or_default();
        if needs_space(last_or_empty(&out), &first) {
            parts.push(Doc::Text(" ".to_owned()));
        }
        parts.push(type_doc(right, source));
    }
    Doc::Concat(parts)
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

    fn grouped_list(items: &[Doc]) -> Doc {
        Doc::Group(Box::new(bracket_list(items, "(", ")")))
    }

    #[test]
    fn bracket_list_stays_inline_when_it_fits() {
        let items = vec![
            Doc::Text("a: i32".to_owned()),
            Doc::Text("b: i32".to_owned()),
        ];
        assert_eq!(render(&grouped_list(&items), 0), "(a: i32, b: i32)");
    }

    #[test]
    fn bracket_list_wraps_over_the_budget_with_a_trailing_comma() {
        let first = "x".repeat(40);
        let second = "y".repeat(40);
        let items = vec![Doc::Text(first.clone()), Doc::Text(second.clone())];
        assert_eq!(
            render(&grouped_list(&items), 0),
            format!("(\n    {first},\n    {second},\n)")
        );
    }

    #[test]
    fn a_header_group_counts_a_trailing_return_type() {
        // The list alone fits, but the whole header (with the return type) does
        // not, so the list must break.
        let items = vec![Doc::Text("a: i32".to_owned())];
        let head = Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text("pub fn f".to_owned()),
            bracket_list(&items, "(", ")"),
            Doc::Text(format!(" -> {}", "R".repeat(80))),
        ])));
        let rendered = render(&head, 0);
        assert!(
            rendered.starts_with("pub fn f(\n    a: i32,\n) -> "),
            "expected a broken list, got {rendered:?}"
        );
    }

    /// Parses a source whose first item carries the attribute under test and
    /// returns the attribute's document and its flat token rendering. The node
    /// is the `attribute_item`, exactly what extraction passes in.
    fn attribute_pair(source: &str) -> (Doc, String) {
        let mut parser = crate::syntax::parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let attribute_item = tree.root_node().named_child(0).expect("an attribute item");
        assert_eq!(attribute_item.kind(), "attribute_item");
        (
            attribute_doc(attribute_item, source),
            render_node(attribute_item, source),
        )
    }

    /// Parses `type __T = <type>;` and returns the right-hand side's type
    /// document together with its flat token rendering.
    fn type_pair(source: &str) -> (Doc, String) {
        let mut parser = crate::syntax::parser().unwrap();
        let tree = parser.parse(source, None).unwrap();
        let item = tree.root_node().named_child(0).expect("a type item");
        let ty = item.child_by_field_name("type").expect("the alias type");
        (type_doc(ty, source), render_node(ty, source))
    }

    #[test]
    fn short_types_render_flat_and_byte_identical() {
        for ty in [
            "Vec<A, B>",
            "(A, B)",
            "&'a Vec<A>",
            "&'static mut Vec<(A, B)>",
            "Result<A, B>",
            "()",
        ] {
            let (doc, flat) = type_pair(&format!("type X = {ty};"));
            assert_eq!(render(&doc, 0), flat, "flat mismatch for {ty}");
        }
    }

    #[test]
    fn function_types_render_flat_and_byte_identical() {
        let (doc, flat) = type_pair("type X = fn(A) -> B;");
        assert_eq!(flat, "fn(A) -> B");
        assert_eq!(render(&doc, 0), flat);

        // The `Fn(...) -> ...` form is the `trait` of a `dyn` type; render that
        // inner `function_type` directly.
        let mut parser = crate::syntax::parser().unwrap();
        let tree = parser.parse("type X = dyn Fn(A) -> B;", None).unwrap();
        let item = tree.root_node().named_child(0).expect("a type item");
        let dynamic = item.child_by_field_name("type").expect("the alias type");
        let inner = dynamic
            .child_by_field_name("trait")
            .expect("the function type");
        assert_eq!(render_node(inner, "type X = dyn Fn(A) -> B;"), "Fn(A) -> B");
        assert_eq!(
            render(&type_doc(inner, "type X = dyn Fn(A) -> B;"), 0),
            "Fn(A) -> B"
        );
    }

    #[test]
    fn a_long_tuple_type_breaks_at_its_items() {
        let first = "A".repeat(40);
        let second = "B".repeat(40);
        let (doc, _) = type_pair(&format!("type X = ({first}, {second});"));
        assert_eq!(
            render(&doc, 0),
            format!("(\n    {first},\n    {second},\n)")
        );
    }

    #[test]
    fn a_long_nested_generic_breaks_at_each_level() {
        let a = "A".repeat(40);
        let b = "B".repeat(40);
        let e = "E".repeat(40);
        let (doc, _) = type_pair(&format!("type X = Result<({a}, {b}), {e}>;"));
        assert_eq!(
            render(&doc, 0),
            format!("Result<\n    (\n        {a},\n        {b},\n    ),\n    {e},\n>")
        );
    }

    #[test]
    fn short_multi_argument_attribute_is_flat_and_byte_identical() {
        let source = "#[arg(long = \"path\", short = 'p')]\nstruct S;";
        let (doc, flat) = attribute_pair(source);
        assert_eq!(flat, "#[arg(long = \"path\", short = 'p')]");
        assert_eq!(render(&doc, 0), flat);
    }

    #[test]
    fn nested_commas_do_not_split_an_attribute() {
        let source = "#[cfg(all(unix, feature = \"x\"))]\nstruct S;";
        let (doc, flat) = attribute_pair(source);
        assert_eq!(render(&doc, 0), flat);
    }

    #[test]
    fn long_multi_argument_attribute_wraps_with_a_trailing_comma() {
        // Flat this attribute is 79 columns, so at depth 1 (four-space indent)
        // it no longer fits and must break.
        let source = "#[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP)]\nstruct S;";
        let (doc, _) = attribute_pair(source);
        assert_eq!(
            render(&doc, 1),
            "    #[command(\n        after_help = FOCUSED_DIFF_HELP,\n        after_long_help = FOCUSED_DIFF_HELP,\n    )]"
        );
    }
}
