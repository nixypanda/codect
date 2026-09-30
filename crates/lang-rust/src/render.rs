//! Canonical rendering of Rust declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs (TECHNICAL_DESIGN.md
//! sections 10 and 12). Node text is never copied verbatim: declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks. Terminal width is not consulted anywhere in this module.

pub(crate) use base::doc::{Doc, render};
use base::{RepoPath, SupportedPath};
use tree_sitter::Node;

use crate::syntax::{
    ATTRIBUTE, BLOCK_COMMENT, BOOLEAN_LITERAL, CHAR_LITERAL, CLOSE_BRACE, CLOSE_BRACKET,
    CLOSE_PAREN, EQUAL, FIELD_ARGUMENTS, FIELD_BODY, FIELD_DECLARATION, FIELD_NAME,
    FIELD_PARAMETERS, FIELD_PATTERN, FIELD_RETURN_TYPE, FIELD_TYPE, FIELD_TYPE_ARGUMENTS,
    FLOAT_LITERAL, FUNCTION_TYPE, GENERIC_TYPE, INTEGER_LITERAL, LINE_COMMENT, NEGATIVE_LITERAL,
    OPEN_BRACE, OPEN_BRACKET, OPEN_PAREN, ORDERED_FIELD_DECLARATION_LIST, PARAMETER, PARAMETERS,
    RAW_STRING_LITERAL, REFERENCE_TYPE, SEMICOLON, STRING_LITERAL, TOKEN_TREE, TRAIT_BOUNDS,
    TUPLE_TYPE, TYPE_PARAMETERS, VISIBILITY_MODIFIER, WHERE_CLAUSE,
};

fn is_comment(kind: &str) -> bool {
    matches!(kind, LINE_COMMENT | BLOCK_COMMENT)
}

/// Literals whose internal bytes must be preserved exactly, including spaces
/// inside string contents and characters.
fn is_atomic_literal(kind: &str) -> bool {
    matches!(
        kind,
        STRING_LITERAL
            | RAW_STRING_LITERAL
            | CHAR_LITERAL
            | INTEGER_LITERAL
            | FLOAT_LITERAL
            | BOOLEAN_LITERAL
            | NEGATIVE_LITERAL
    )
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

fn last_or_empty(tokens: &[String]) -> &str {
    tokens.last().map(String::as_str).unwrap_or("")
}

/// Canonical renderer for Rust declarations.
///
/// Holds the source and its path so declaration fragments are rebuilt from the
/// parsed tree, never from raw source slices (except atomic literals).
pub(crate) struct Renderer<'a> {
    path: &'a SupportedPath,
    source: &'a str,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(path: &'a SupportedPath, source: &'a str) -> Self {
        Self { path, source }
    }

    pub(crate) fn path(&self) -> &RepoPath {
        self.path.path()
    }

    /// The source slice a node spans, or `""` when the range is out of bounds.
    pub(crate) fn slice(&self, node: Node<'_>) -> &'a str {
        self.source.get(node.byte_range()).unwrap_or("")
    }

    fn push_tokens(&self, node: Node<'_>, out: &mut Vec<String>) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let kind = current.kind();
            if is_comment(kind) {
                continue;
            }
            if current.child_count() == 0 || is_atomic_literal(kind) {
                if let Some(text) = self.source.get(current.byte_range()) {
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

    pub(crate) fn tokens(&self, node: Node<'_>) -> Vec<String> {
        let mut out = Vec::new();
        self.push_tokens(node, &mut out);
        out
    }

    pub(crate) fn render_node(&self, node: Node<'_>) -> String {
        join(&self.tokens(node))
    }

    fn child_by_field_name<'t>(&self, node: Node<'t>, name: &str) -> Option<Node<'t>> {
        node.child_by_field_name(name)
    }

    pub(crate) fn child_of_kind<'t>(&self, node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|child| child.kind() == kind)
    }

    /// One element of a declaration header: either flattened tokens, a
    /// recursively rendered type, or a bracketed list that is allowed to wrap.
    /// Renders the declaration prefix up to the first child in `stop_kinds`,
    /// dropping statement terminators that the caller re-emits.
    ///
    /// Parameter and type-parameter lists become groups that wrap at
    /// [`LINE_WIDTH`]; every other token keeps the fixed spacing rules, so a
    /// header that fits is byte-for-byte the old flat output. The caller groups
    /// it together with whatever shares its first line, so the fit check counts
    /// the terminator or opening brace too.
    pub(crate) fn header(&self, node: Node<'_>, stop_kinds: &[&str]) -> Doc {
        let elements = self.header_elements(node, stop_kinds);
        elements_doc(&elements)
    }

    fn header_elements(&self, node: Node<'_>, stop_kinds: &[&str]) -> Vec<Elem> {
        let mut elements = Vec::new();
        let return_type = self.child_by_field_name(node, FIELD_RETURN_TYPE);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if stop_kinds.contains(&kind) {
                break;
            }
            if kind == SEMICOLON {
                continue;
            }
            if Some(child.id()) == return_type.map(|node| node.id()) {
                let leaves = self.tokens(child);
                if let (Some(first), Some(last)) = (leaves.first(), leaves.last()) {
                    elements.push(Elem::Type {
                        doc: self.type_doc(child),
                        first: first.clone(),
                        last: last.clone(),
                    });
                    continue;
                }
            }
            if kind == PARAMETERS {
                elements.push(self.list_elem(child, "(", ")"));
            } else if kind == TYPE_PARAMETERS {
                elements.push(self.list_elem(child, "<", ">"));
            } else {
                for token in self.tokens(child) {
                    elements.push(Elem::Token(token));
                }
            }
        }
        elements
    }

    fn list_elem(&self, node: Node<'_>, open: &'static str, close: &'static str) -> Elem {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == PARAMETER
                && self.child_by_field_name(child, FIELD_PATTERN).is_some()
                && self.child_by_field_name(child, FIELD_TYPE).is_some()
            {
                items.push(self.parameter_doc(child));
            } else {
                items.push(Doc::Text(self.render_node(child)));
            }
        }
        Elem::List { items, open, close }
    }

    /// A named parameter's document: its pattern, `: `, and its recursively
    /// rendered type. A parameter type can therefore break inside an already-broken
    /// parameter list. Anything else (notably `self`) is rendered flat.
    fn parameter_doc(&self, node: Node<'_>) -> Doc {
        match self.child_by_field_name(node, FIELD_TYPE) {
            Some(ty) => self.prefix_then_type(node, ty),
            None => Doc::Text(self.render_node(node)),
        }
    }

    /// A recursive type document. Only the node kinds below are rebuilt; every
    /// other kind falls back to the flat token rendering, so an unrecognized type
    /// cannot change its flat output. Each bracket list is grouped so it re-decides
    /// its own fit after an enclosing list breaks.
    pub(crate) fn type_doc(&self, node: Node<'_>) -> Doc {
        match node.kind() {
            GENERIC_TYPE => {
                let mut parts = Vec::new();
                if let Some(ty) = self.child_by_field_name(node, FIELD_TYPE) {
                    parts.push(self.type_doc(ty));
                }
                if let Some(arguments) = self.child_by_field_name(node, FIELD_TYPE_ARGUMENTS) {
                    let items = self.named_type_children(arguments);
                    parts.push(bracket_list(&items, "<", ">"));
                }
                Doc::Group(Box::new(Doc::Concat(parts)))
            }
            TUPLE_TYPE => {
                let items = self.named_type_children(node);
                Doc::Group(Box::new(bracket_list(&items, "(", ")")))
            }
            REFERENCE_TYPE => {
                let Some(inner) = self.child_by_field_name(node, FIELD_TYPE) else {
                    return Doc::Text(self.render_node(node));
                };
                let mut prefix_tokens = Vec::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.id() == inner.id() {
                        break;
                    }
                    self.push_tokens(child, &mut prefix_tokens);
                }
                let mut parts = vec![Doc::Text(join(&prefix_tokens))];
                let first = self.tokens(inner).into_iter().next().unwrap_or_default();
                if needs_space(last_or_empty(&prefix_tokens), &first) {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                parts.push(self.type_doc(inner));
                Doc::Concat(parts)
            }
            FUNCTION_TYPE => {
                let parameters = self.child_by_field_name(node, FIELD_PARAMETERS);
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
                    self.push_tokens(child, &mut prefix);
                    parts.push(Doc::Text(join(&prefix)));
                }
                if let Some(parameters) = parameters {
                    let items = self.named_type_children(parameters);
                    parts.push(bracket_list(&items, "(", ")"));
                }
                if let Some(return_type) = self.child_by_field_name(node, FIELD_RETURN_TYPE) {
                    parts.push(Doc::Text(" -> ".to_owned()));
                    parts.push(self.type_doc(return_type));
                }
                Doc::Group(Box::new(Doc::Concat(parts)))
            }
            _ => Doc::Text(self.render_node(node)),
        }
    }

    fn named_type_children(&self, node: Node<'_>) -> Vec<Doc> {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(self.type_doc(child));
        }
        items
    }

    pub(crate) fn where_clause_text(&self, node: Node<'_>) -> Option<String> {
        self.child_of_kind(node, WHERE_CLAUSE)
            .map(|clause| self.render_node(clause))
    }

    /// The depth-1 comma-separated argument items of an attribute's `token_tree`,
    /// or `None` when there is no argument list or it has no top-level comma.
    ///
    /// The `token_tree` includes its own outer delimiters as direct children; they
    /// are dropped so the caller can re-emit the list through [`bracket_list`].
    /// Nested token trees are single children, so a comma inside one never splits.
    fn attribute_argument_items(&self, attribute: Node<'_>) -> Option<Vec<String>> {
        let arguments = attribute.child_by_field_name(FIELD_ARGUMENTS)?;
        if arguments.kind() != TOKEN_TREE {
            return None;
        }
        let mut cursor = arguments.walk();
        let children: Vec<Node<'_>> = arguments.children(&mut cursor).collect();
        let inner: &[Node<'_>] = match (children.first(), children.last()) {
            (Some(first), Some(last))
                if matches!(
                    (first.kind(), last.kind()),
                    (OPEN_PAREN, CLOSE_PAREN)
                        | (OPEN_BRACKET, CLOSE_BRACKET)
                        | (OPEN_BRACE, CLOSE_BRACE)
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
                OPEN_PAREN | OPEN_BRACKET | OPEN_BRACE => depth += 1,
                CLOSE_PAREN | CLOSE_BRACKET | CLOSE_BRACE => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
            self.push_tokens(*child, &mut segment);
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
    pub(crate) fn attribute_doc(&self, attribute_item: Node<'_>) -> Doc {
        // Extraction passes the `attribute_item`, whose only named child is the
        // `attribute` carrying the path and the argument token tree.
        let attribute = if attribute_item.kind() == ATTRIBUTE {
            attribute_item
        } else {
            match self.child_of_kind(attribute_item, ATTRIBUTE) {
                Some(attribute) => attribute,
                None => return Doc::Text(self.render_node(attribute_item)),
            }
        };
        let arguments = attribute.child_by_field_name(FIELD_ARGUMENTS);
        let mut cursor = attribute.walk();
        let path = attribute
            .named_children(&mut cursor)
            .find(|child| Some(*child) != arguments);
        let (Some(path), Some(items)) = (path, self.attribute_argument_items(attribute)) else {
            return Doc::Text(self.render_node(attribute_item));
        };
        let arguments: Vec<Doc> = items.into_iter().map(Doc::Text).collect();
        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text("#[".to_owned()),
            Doc::Text(self.render_node(path)),
            bracket_list(&arguments, "(", ")"),
            Doc::Text("]".to_owned()),
        ])))
    }

    fn field_docs(&self, list: Node<'_>) -> Vec<Doc> {
        let mut out = Vec::new();
        let mut cursor = list.walk();
        for child in list.children(&mut cursor) {
            if child.kind() == FIELD_DECLARATION {
                out.push(self.field_doc(child));
            }
        }
        out
    }

    /// A named field's document: everything before the declared type (visibility,
    /// name, and `:`), then the recursively rendered type. The flat rendering is
    /// byte-identical to the previous `render_node(field)`, but a long type can now
    /// break.
    pub(crate) fn field_doc(&self, node: Node<'_>) -> Doc {
        match self.child_by_field_name(node, FIELD_TYPE) {
            Some(ty) => self.prefix_then_type(node, ty),
            None => Doc::Text(self.render_node(node)),
        }
    }

    /// Everything before `ty` (its sibling tokens are rendered with [`join`]), then
    /// a space when required, then the recursively rendered type. This keeps the
    /// prefix byte-identical to [`render_node`] while letting `ty` break.
    fn prefix_then_type(&self, node: Node<'_>, ty: Node<'_>) -> Doc {
        let mut prefix = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.id() == ty.id() {
                break;
            }
            self.push_tokens(child, &mut prefix);
        }
        let mut parts = vec![Doc::Text(join(&prefix))];
        let first = self.tokens(ty).into_iter().next().unwrap_or_default();
        if needs_space(last_or_empty(&prefix), &first) {
            parts.push(Doc::Text(" ".to_owned()));
        }
        parts.push(self.type_doc(ty));
        Doc::Concat(parts)
    }

    /// A variant body as an ungrouped document. A struct-like variant's field list
    /// is a brace list the caller can group with the trailing comma.
    pub(crate) fn variant_doc(&self, variant: Node<'_>) -> Doc {
        let name = self
            .child_by_field_name(variant, FIELD_NAME)
            .map(|node| self.render_node(node))
            .unwrap_or_default();

        match self.child_by_field_name(variant, FIELD_BODY) {
            None => Doc::Text(self.render_node(variant)),
            Some(body) if body.kind() == ORDERED_FIELD_DECLARATION_LIST => {
                Doc::Text(format!("{name}{}", self.render_node(body)))
            }
            Some(body) => {
                let fields = self.field_docs(body);
                if fields.is_empty() {
                    Doc::Text(format!("{name} {{}}"))
                } else {
                    Doc::Concat(vec![Doc::Text(name), brace_list(&fields)])
                }
            }
        }
    }

    /// `type Item = RightHandSide`, without the terminator or where clause.
    pub(crate) fn type_alias_text(&self, node: Node<'_>) -> Doc {
        let mut out = Vec::new();
        if let Some(visibility) = self.child_of_kind(node, VISIBILITY_MODIFIER) {
            self.push_tokens(visibility, &mut out);
        }
        out.push("type".to_owned());
        if let Some(name) = self.child_by_field_name(node, FIELD_NAME) {
            self.push_tokens(name, &mut out);
        }
        if let Some(parameters) = self.child_of_kind(node, TYPE_PARAMETERS) {
            self.push_tokens(parameters, &mut out);
        }
        out.push(EQUAL.to_owned());
        let mut parts = vec![Doc::Text(join(&out))];
        if let Some(right) = self.child_by_field_name(node, FIELD_TYPE) {
            let first = self.tokens(right).into_iter().next().unwrap_or_default();
            if needs_space(last_or_empty(&out), &first) {
                parts.push(Doc::Text(" ".to_owned()));
            }
            parts.push(self.type_doc(right));
        }
        Doc::Concat(parts)
    }

    /// `type Error: Bound + Bound`, without the terminator or where clause.
    pub(crate) fn associated_type_text(&self, node: Node<'_>) -> String {
        let mut out = vec!["type".to_owned()];
        if let Some(name) = self.child_by_field_name(node, FIELD_NAME) {
            self.push_tokens(name, &mut out);
        }
        if let Some(parameters) = self.child_of_kind(node, TYPE_PARAMETERS) {
            self.push_tokens(parameters, &mut out);
        }
        if let Some(bounds) = self.child_of_kind(node, TRAIT_BOUNDS) {
            self.push_tokens(bounds, &mut out);
        }
        join(&out)
    }

    /// A constant or static declaration with its initializer removed.
    pub(crate) fn constant_text(&self, node: Node<'_>) -> String {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            if kind == EQUAL {
                break;
            }
            if kind == SEMICOLON {
                continue;
            }
            self.push_tokens(child, &mut out);
        }
        join(&out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax;
    use base::RepoPath;

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

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
        let path = supported("src/lib.rs");
        let tree = syntax::parse(source, &path).expect("the source parses");
        let renderer = Renderer::new(&path, source);
        let attribute_item = tree.root_node().named_child(0).expect("an attribute item");
        assert_eq!(attribute_item.kind(), "attribute_item");
        (
            renderer.attribute_doc(attribute_item),
            renderer.render_node(attribute_item),
        )
    }

    /// Parses `type __T = <type>;` and returns the right-hand side's type
    /// document together with its flat token rendering.
    fn type_pair(source: &str) -> (Doc, String) {
        let path = supported("src/lib.rs");
        let tree = syntax::parse(source, &path).expect("the source parses");
        let renderer = Renderer::new(&path, source);
        let item = tree.root_node().named_child(0).expect("a type item");
        let ty = item.child_by_field_name("type").expect("the alias type");
        (renderer.type_doc(ty), renderer.render_node(ty))
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
        let source = "type X = dyn Fn(A) -> B;";
        let path = supported("src/lib.rs");
        let tree = syntax::parse(source, &path).expect("the source parses");
        let renderer = Renderer::new(&path, source);
        let item = tree.root_node().named_child(0).expect("a type item");
        let dynamic = item.child_by_field_name("type").expect("the alias type");
        let inner = dynamic
            .child_by_field_name("trait")
            .expect("the function type");
        assert_eq!(renderer.render_node(inner), "Fn(A) -> B");
        assert_eq!(render(&renderer.type_doc(inner), 0), "Fn(A) -> B");
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
