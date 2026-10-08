//! Canonical rendering of Python declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs. Declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks; source slices are used only for atomic literals such as strings,
//! where internal bytes carry meaning. Terminal width is never consulted.

pub(crate) use base::doc::{Doc, render};
use base::{ProjectionError, SupportedPath};
use tree_sitter::Node;

use crate::syntax::{self, field, node};

// One leaf token with explicit glue flags. `glue_before` and `glue_after`
// suppress the default space on that side, which is how `keyword=value` is
// written without spaces while `name: T = default` keeps them.
#[derive(Clone, Debug)]
struct Tok {
    text: String,
    glue_before: bool,
    glue_after: bool,
}

impl Tok {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            glue_before: false,
            glue_after: false,
        }
    }

    fn glued(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            glue_before: true,
            glue_after: true,
        }
    }
}

pub(crate) struct Renderer<'a> {
    path: &'a SupportedPath,
    source: &'a str,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(path: &'a SupportedPath, source: &'a str) -> Self {
        Self { path, source }
    }

    // The path and its derived language, for building a [`ProjectionError`].
    pub(crate) fn supported_path(&self) -> &SupportedPath {
        self.path
    }

    pub(crate) fn slice(&self, node: Node<'_>) -> &'a str {
        self.source.get(node.byte_range()).unwrap_or("")
    }

    pub(crate) fn invariant<T>(
        &self,
        node: Node<'_>,
        detail: impl Into<String>,
    ) -> Result<T, ProjectionError> {
        Err(ProjectionError::AstInvariant {
            path: self.path.clone(),
            range: syntax::node_span(node),
            detail: detail.into(),
        })
    }

    pub(crate) fn field<'n>(&self, node: Node<'n>, name: &str) -> Option<Node<'n>> {
        node.child_by_field_name(name)
    }

    pub(crate) fn field_text(&self, node: Node<'_>, field: &str) -> Option<String> {
        self.field(node, field).map(|child| self.node_text(child))
    }

    // The first child of `kind`, named or not. Python needs this for anonymous
    // keyword tokens such as `async`.
    pub(crate) fn child_of_kind_any<'t>(&self, node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|child| child.kind() == kind)
    }

    fn push_tokens(&self, node: Node<'_>, out: &mut Vec<Tok>) {
        let kind = node.kind();
        if kind == node::COMMENT {
            return;
        }
        if kind == node::STRING {
            out.push(Tok::plain(self.slice(node)));
            return;
        }
        if kind == node::KEYWORD_ARGUMENT {
            if let Some(name) = node.child_by_field_name(field::NAME) {
                self.push_tokens(name, out);
            }
            out.push(Tok::glued("="));
            if let Some(value) = node.child_by_field_name(field::VALUE) {
                self.push_tokens(value, out);
            }
            return;
        }
        // An unannotated default is written `name=value`; an annotated default
        // is written `name: T = value` and is handled by the generic path.
        if kind == node::DEFAULT_PARAMETER {
            if let Some(name) = node.child_by_field_name(field::NAME) {
                self.push_tokens(name, out);
            }
            out.push(Tok::glued("="));
            if let Some(value) = node.child_by_field_name(field::VALUE) {
                self.push_tokens(value, out);
            }
            return;
        }
        if node.child_count() == 0 {
            out.push(Tok::plain(self.slice(node)));
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.push_tokens(child, out);
        }
    }

    pub(crate) fn node_text(&self, node: Node<'_>) -> String {
        let mut tokens = Vec::new();
        self.push_tokens(node, &mut tokens);
        join(&tokens)
    }

    // A decorator, with a breakable argument list when its expression is a
    // call. A bare decorator (`@property`) or any other expression stays flat.
    //
    // The `decorator` node has no fields; its first named child is the
    // expression. When that expression is a `call`, the call's `arguments`
    // (`argument_list`) named children are the items, rendered through
    // [`Renderer::bracket_list`] so a long decorator breaks one argument per
    // indented line with a trailing comma while a short one stays inline. Each
    // item is a [`Renderer::value_doc`] so a collection argument breaks at its
    // own column once the decorator itself has broken.
    pub(crate) fn decorator_doc(&self, node: Node<'_>) -> Doc {
        let mut cursor = node.walk();
        let expression = node.named_children(&mut cursor).next();
        if let Some(call) = expression.filter(|child| child.kind() == node::CALL)
            && let (Some(function), Some(arguments)) = (
                call.child_by_field_name(field::FUNCTION),
                call.child_by_field_name(field::ARGUMENTS),
            )
        {
            return Doc::Group(Box::new(Doc::Concat(vec![
                Doc::Text("@".to_owned()),
                self.value_doc(function),
                self.argument_list_doc(arguments),
            ])));
        }
        Doc::Text(self.node_text(node))
    }

    // A `generator_expression` (from `@deco(x for x in y)`) also satisfies the
    // `arguments` field but has no comma-separated items, so it stays flat
    // rather than breaking into text that is not a Python expression.
    fn argument_list_doc(&self, node: Node<'_>) -> Doc {
        if node.kind() != node::ARGUMENT_LIST {
            return Doc::Text(self.node_text(node));
        }
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(self.value_doc(child));
        }
        self.bracket_list(items, "(", ")")
    }

    // A bracketed list body: inline while the enclosing [`Doc::Group`] fits,
    // one item per indented line when it breaks.
    //
    // `open`/`close` are the delimiters (`()` for parameters, `[]` for type
    // parameters). The trailing comma is emitted only when the group breaks, so
    // appending an item changes exactly one line. This is deliberately ungrouped
    // so the caller can group the whole declaration header, letting the fit
    // check see the return type and other trailing text.
    //
    // Items are [`Doc`]s so a nested list re-decides at its own column after the
    // outer list breaks. The last item carries a [`Doc::Reserve`] for the
    // trailing comma that lands on its line.
    pub(crate) fn bracket_list(&self, items: Vec<Doc>, open: &str, close: &str) -> Doc {
        if items.is_empty() {
            return Doc::Text(format!("{open}{close}"));
        }

        let mut inner: Vec<Doc> = vec![Doc::SoftNil];
        let last = items.len() - 1;
        for (index, item) in items.into_iter().enumerate() {
            if index == last {
                inner.push(Doc::Reserve(1));
            }
            if index > 0 {
                inner.push(Doc::Text(",".to_owned()));
                inner.push(Doc::SoftLine);
            }
            inner.push(item);
        }
        inner.push(Doc::Broken(","));

        Doc::Concat(vec![
            Doc::Text(open.to_owned()),
            Doc::Indent(Box::new(Doc::Concat(inner))),
            Doc::SoftNil,
            Doc::Text(close.to_owned()),
        ])
    }

    // [`Renderer::bracket_list`] over a node's named children rendered as flat
    // text, for lists whose items have no internal break points (superclasses,
    // type parameters).
    pub(crate) fn node_bracket_list(&self, node: Node<'_>, open: &str, close: &str) -> Doc {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(Doc::Text(self.node_text(child)));
        }
        self.bracket_list(items, open, close)
    }

    // The expression-position counterpart of [`Renderer::type_doc`]. Only the
    // bracketed kinds decorators and defaults use are modeled, so an unmodeled
    // expression cannot regress and stays byte-identical to
    // [`Renderer::node_text`].
    //
    // Rebuilding a list normalizes a dangling trailing comma away, keeping the
    // projection invariant under that formatting-only edit, as parameter lists
    // already are.
    pub(crate) fn value_doc(&self, node: Node<'_>) -> Doc {
        match node.kind() {
            node::LIST => self.grouped_bracket_doc(node, "[", "]"),
            node::SET | node::DICTIONARY => self.grouped_bracket_doc(node, "{", "}"),
            node::TUPLE => self.tuple_doc(node),
            node::CALL => self.call_doc(node),
            node::SUBSCRIPT => self.subscript_doc(node),
            node::PARENTHESIZED_EXPRESSION => self.parenthesized_doc(node),
            node::PAIR => self.pair_doc(node),
            node::KEYWORD_ARGUMENT => self.keyword_argument_doc(node),
            node::LIST_SPLAT => self.splat_doc(node, "*"),
            node::DICTIONARY_SPLAT => self.splat_doc(node, "**"),
            _ => Doc::Text(self.node_text(node)),
        }
    }

    // A one-element tuple's comma is what makes it a tuple at all, and a tuple
    // written without parentheses has no list to break.
    fn tuple_doc(&self, node: Node<'_>) -> Doc {
        let Some(items) = self.bracketed_items(node, "(", ")") else {
            return Doc::Text(self.node_text(node));
        };
        if items.len() == 1 && self.has_trailing_comma(node) {
            return Doc::Text(self.node_text(node));
        }
        self.grouped_bracket_doc(node, "(", ")")
    }

    fn call_doc(&self, node: Node<'_>) -> Doc {
        let (Some(function), Some(arguments)) = (
            node.child_by_field_name(field::FUNCTION),
            node.child_by_field_name(field::ARGUMENTS),
        ) else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Group(Box::new(Doc::Concat(vec![
            self.value_doc(function),
            self.argument_list_doc(arguments),
        ])))
    }

    // The grammar repeats the `subscript` field for every comma-separated
    // element, so the indices are read from the node's children rather than
    // through `child_by_field_name`, which sees only the first. A slice keeps
    // its colon, which a comma-separated rebuild would drop.
    fn subscript_doc(&self, node: Node<'_>) -> Doc {
        let Some(value) = node.child_by_field_name(field::VALUE) else {
            return Doc::Text(self.node_text(node));
        };
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        let mut items = Vec::new();
        for child in children.iter().filter(|child| child.id() != value.id()) {
            if !child.is_named() {
                continue;
            }
            if child.kind() == node::SLICE {
                return Doc::Text(self.node_text(node));
            }
            items.push(self.value_doc(*child));
        }
        Doc::Group(Box::new(Doc::Concat(vec![
            self.value_doc(value),
            self.bracket_list(items, "[", "]"),
        ])))
    }

    fn parenthesized_doc(&self, node: Node<'_>) -> Doc {
        let mut cursor = node.walk();
        let inner: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        let [only] = inner.as_slice() else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text("(".to_owned()),
            self.value_doc(*only),
            Doc::Text(")".to_owned()),
        ])))
    }

    fn pair_doc(&self, node: Node<'_>) -> Doc {
        let (Some(key), Some(value)) = (
            node.child_by_field_name(field::KEY),
            node.child_by_field_name(field::VALUE),
        ) else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Concat(vec![
            self.value_doc(key),
            Doc::Text(": ".to_owned()),
            self.value_doc(value),
        ])
    }

    // The equals stays glued, as in [`Renderer::push_tokens`].
    fn keyword_argument_doc(&self, node: Node<'_>) -> Doc {
        let (Some(name), Some(value)) = (
            node.child_by_field_name(field::NAME),
            node.child_by_field_name(field::VALUE),
        ) else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Concat(vec![
            self.value_doc(name),
            Doc::Text("=".to_owned()),
            self.value_doc(value),
        ])
    }

    fn splat_doc(&self, node: Node<'_>, star: &str) -> Doc {
        let mut cursor = node.walk();
        let named: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        let [only] = named.as_slice() else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Concat(vec![Doc::Text(star.to_owned()), self.value_doc(*only)])
    }

    // [`Renderer::bracket_list`] over a delimited node's named children,
    // grouped so it re-decides at its own column.
    fn grouped_bracket_doc(&self, node: Node<'_>, open: &str, close: &str) -> Doc {
        let Some(items) = self.bracketed_items(node, open, close) else {
            return Doc::Text(self.node_text(node));
        };
        let items = items.iter().map(|child| self.value_doc(*child)).collect();
        Doc::Group(Box::new(self.bracket_list(items, open, close)))
    }

    // The named children between `open` and `close`, or `None` when the node is
    // not delimited by them.
    fn bracketed_items<'t>(
        &self,
        node: Node<'t>,
        open: &str,
        close: &str,
    ) -> Option<Vec<Node<'t>>> {
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        let first = children.first()?;
        let last = children.last()?;
        if self.slice(*first) != open || self.slice(*last) != close {
            return None;
        }
        Some(
            children[1..children.len() - 1]
                .iter()
                .filter(|child| child.is_named())
                .copied()
                .collect(),
        )
    }

    fn has_trailing_comma(&self, node: Node<'_>) -> bool {
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        children.len() >= 3 && self.slice(children[children.len() - 2]) == ","
    }

    // A function's parameter list, with each parameter rendered by
    // [`Renderer::parameter_doc`] so a long parameter type can break once the
    // parameter list itself breaks.
    pub(crate) fn parameters_doc(&self, node: Node<'_>) -> Doc {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(self.parameter_doc(child));
        }
        self.bracket_list(items, "(", ")")
    }

    // A parameter, keeping its declared type and its default as nested [`Doc`]s.
    //
    // Only a parameter that declares a `type` or a `value` field is split; a
    // splat pattern (`*args: T`, `**kwargs: T`), whose type hangs off a wrapper
    // node, stays flat. The tokens between the split points are rendered
    // separately with the boundary space restored, which is what keeps the flat
    // form byte-identical to [`Renderer::node_text`].
    fn parameter_doc(&self, node: Node<'_>) -> Doc {
        let mut breakables: Vec<(Node<'_>, bool)> = Vec::new();
        for (name, is_type) in [(field::TYPE, true), (field::VALUE, false)] {
            if let Some(child) = node.child_by_field_name(name) {
                breakables.push((child, is_type));
            }
        }
        if breakables.is_empty() {
            return Doc::Text(self.node_text(node));
        }

        // A split point that contributes no tokens of its own would silently
        // drop out of the flat form.
        let mut nested_tokens: Vec<Vec<Tok>> = Vec::new();
        for (child, _) in &breakables {
            let mut tokens = Vec::new();
            self.push_tokens(*child, &mut tokens);
            if tokens.is_empty() {
                return Doc::Text(self.node_text(node));
            }
            nested_tokens.push(tokens);
        }

        // An unannotated default is written `name=value`, so its equals is glued
        // as in [`Renderer::push_tokens`].
        let glue_equals = node.kind() == node::DEFAULT_PARAMETER;

        let mut docs = Vec::new();
        let mut segment: Vec<Tok> = Vec::new();
        let mut previous: Option<&[Tok]> = None;
        let mut split = 0usize;
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        for child in &children {
            let Some(slot) = breakables
                .iter()
                .position(|(breakable, _)| breakable.id() == child.id())
            else {
                if glue_equals && child.kind() == "=" {
                    segment.push(Tok::glued("="));
                } else {
                    self.push_tokens(*child, &mut segment);
                }
                continue;
            };

            docs.push(Doc::Text(join_segment(
                &segment,
                previous,
                &nested_tokens[slot],
            )));
            let (_, is_type) = breakables[slot];
            docs.push(if is_type {
                self.type_doc(*child)
            } else {
                self.value_doc(*child)
            });
            previous = Some(&nested_tokens[slot]);
            split = slot + 1;
            segment.clear();
        }
        let tail = nested_tokens.get(split).map_or(&[][..], |next| &next[..]);
        docs.push(Doc::Text(join_segment(&segment, previous, tail)));

        Doc::Concat(docs)
    }

    // A recursive type [`Doc`].
    //
    // Only the node kinds that appear in Python type positions are handled:
    // the `type` wrapper, `generic_type` subscripts, and `binary_operator`
    // unions. Every other kind falls back to flat text, so an unmodeled type
    // cannot regress and always renders byte-identically to
    // [`Renderer::node_text`].
    //
    // This grammar (tree-sitter-python 0.25) represents annotations with a
    // `type` wrapper around a `generic_type` (`list[int]`) rather than a
    // `subscript`; a bare `subscript` in an expression position is left flat.
    pub(crate) fn type_doc(&self, node: Node<'_>) -> Doc {
        match node.kind() {
            node::TYPE => {
                let mut cursor = node.walk();
                let named: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
                match named.as_slice() {
                    [only] => self.type_doc(*only),
                    _ => Doc::Text(self.node_text(node)),
                }
            }
            node::GENERIC_TYPE => self.generic_type_doc(node),
            node::BINARY_OPERATOR => self.binary_operator_type_doc(node),
            _ => Doc::Text(self.node_text(node)),
        }
    }

    // A subscript type such as `dict[str, int]` or `tuple[A, B]`.
    //
    // The value (`dict`, `tuple`, or a dotted attribute) stays flat and each
    // bracketed argument group becomes a breakable list whose items are
    // recursively typed. The whole node is grouped so it re-decides at its own
    // column after an enclosing parameter list breaks.
    fn generic_type_doc(&self, node: Node<'_>) -> Doc {
        let mut cursor = node.walk();
        let mut named = node.named_children(&mut cursor);
        let Some(value) = named.next() else {
            return Doc::Text(self.node_text(node));
        };
        let mut parts = vec![Doc::Text(self.node_text(value))];
        for parameter in named {
            if parameter.kind() != node::TYPE_PARAMETER {
                return Doc::Text(self.node_text(node));
            }
            let mut cursor = parameter.walk();
            let items: Vec<Doc> = parameter
                .named_children(&mut cursor)
                .map(|child| self.type_doc(child))
                .collect();
            parts.push(self.bracket_list(items, "[", "]"));
        }
        Doc::Group(Box::new(Doc::Concat(parts)))
    }

    fn binary_operator_type_doc(&self, node: Node<'_>) -> Doc {
        let is_union = node
            .child_by_field_name("operator")
            .is_some_and(|operator| self.slice(operator) == "|");
        if !is_union {
            return Doc::Text(self.node_text(node));
        }

        let mut operands = Vec::new();
        self.collect_union_operands(node, &mut operands);
        if operands.len() < 2 {
            return Doc::Text(self.node_text(node));
        }

        let mut docs: Vec<Doc> = operands
            .into_iter()
            .map(|operand| self.type_doc(operand))
            .collect();
        let first = docs.remove(0);
        let mut rest = Vec::new();
        for atom in docs {
            rest.push(Doc::SoftLine);
            rest.push(Doc::Text("| ".to_owned()));
            rest.push(atom);
        }
        Doc::Group(Box::new(Doc::Concat(vec![
            first,
            Doc::Indent(Box::new(Doc::Concat(rest))),
        ])))
    }

    fn collect_union_operands<'t>(&self, node: Node<'t>, out: &mut Vec<Node<'t>>) {
        let is_union = node.kind() == node::BINARY_OPERATOR
            && node
                .child_by_field_name("operator")
                .is_some_and(|operator| self.slice(operator) == "|");
        if !is_union {
            out.push(node);
            return;
        }
        if let Some(left) = node.child_by_field_name("left") {
            self.collect_union_operands(left, out);
        }
        if let Some(right) = node.child_by_field_name("right") {
            self.collect_union_operands(right, out);
        }
    }
}

// Spacing is a function of the two adjacent leaf tokens, never of source
// whitespace.
fn join(tokens: &[Tok]) -> String {
    let mut text = String::new();
    let mut previous: Option<&Tok> = None;
    for token in tokens {
        if let Some(previous) = previous
            && !previous.glue_after
            && !token.glue_before
            && needs_space(&previous.text, &token.text)
        {
            text.push(' ');
        }
        text.push_str(&token.text);
        previous = Some(token);
    }
    text
}

// The flat text of the tokens between two split points, with the boundary space
// restored so it matches what [`join`] would produce. `previous` and `next` are
// the split points on either side, either of which may be absent.
fn join_segment(segment: &[Tok], previous: Option<&[Tok]>, next: &[Tok]) -> String {
    let mut text = String::new();
    if let (Some(left), Some(right)) = (previous.and_then(<[Tok]>::last), segment.first())
        && boundary_space(left, right)
    {
        text.push(' ');
    }
    text.push_str(&join(segment));
    if let (Some(last), Some(right)) = (segment.last(), next.first())
        && boundary_space(last, right)
    {
        text.push(' ');
    }
    text
}

fn boundary_space(left: &Tok, right: &Tok) -> bool {
    !left.glue_after && !right.glue_before && needs_space(&left.text, &right.text)
}

fn needs_space(previous: &str, next: &str) -> bool {
    if previous.is_empty() || next.is_empty() {
        return false;
    }

    // Punctuation that always attaches to the preceding token.
    if matches!(next, ")" | "]" | "}" | "," | ";" | ":" | ".") {
        return false;
    }

    // An opening bracket normally attaches to what precedes it (calls and
    // subscripts), unless it follows a separator or operator.
    if matches!(next, "(" | "[" | "{") {
        return matches!(previous, "," | ";" | ":" | "=" | "->" | "|");
    }

    // Opening delimiters and prefix punctuation attach to what follows them.
    if matches!(previous, "(" | "[" | "{" | ".") {
        return false;
    }
    if matches!(previous, "*" | "**" | "@") {
        return false;
    }

    // Separators and infix operators are surrounded by one space.
    if matches!(previous, "," | ";" | ":") {
        return true;
    }
    if matches!(
        previous,
        "=" | "->"
            | "|"
            | ":="
            | "=="
            | "!="
            | "<"
            | ">"
            | "<="
            | ">="
            | "+"
            | "-"
            | "/"
            | "//"
            | "%"
            | "&"
            | "^"
            | "<<"
            | ">>"
            | "and"
            | "or"
            | "in"
            | "is"
            | "not"
    ) {
        return true;
    }
    if matches!(next, "=" | "->" | "|" | ":=") {
        return true;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{LINE_WIDTH, RepoPath};

    fn supported(raw: impl AsRef<[u8]>) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    fn spaced(parts: &[&str]) -> String {
        let tokens: Vec<Tok> = parts.iter().map(|part| Tok::plain(*part)).collect();
        join(&tokens)
    }

    #[test]
    fn spacing_normalizes_python_fragments() {
        assert_eq!(spaced(&["[", "int", "]"]), "[int]");
        assert_eq!(spaced(&["list", "[", "str", "]"]), "list[str]");
        assert_eq!(spaced(&["int", "|", "None"]), "int | None");
        assert_eq!(
            spaced(&["dict", "[", "str", ",", "int", "]"]),
            "dict[str, int]"
        );
        assert_eq!(spaced(&["a", ".", "b", "(", "c", ")"]), "a.b(c)");
        assert_eq!(spaced(&["def", "f"]), "def f");
    }

    #[test]
    fn glued_equals_has_no_spaces() {
        let tokens = vec![Tok::plain("frozen"), Tok::glued("="), Tok::plain("True")];
        assert_eq!(join(&tokens), "frozen=True");
    }

    fn list(items: &[String]) -> Doc {
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
            Doc::Text("(".to_owned()),
            Doc::Indent(Box::new(Doc::Concat(inner))),
            Doc::SoftNil,
            Doc::Text(")".to_owned()),
        ])))
    }

    #[test]
    fn list_stays_inline_when_it_fits_the_budget() {
        let items = vec!["a: int".to_owned(), "b: int".to_owned()];
        assert_eq!(render(&list(&items), 0), "(a: int, b: int)");
    }

    #[test]
    fn list_breaks_one_item_per_line_over_the_budget() {
        let first = "x".repeat(40);
        let second = "y".repeat(40);
        // Flat width is 1 + 40 + 1 + 1 + 40 + 1 = 84, over LINE_WIDTH.
        let items = vec![first.clone(), second.clone()];
        assert_eq!(
            render(&list(&items), 0),
            format!("(\n    {first},\n    {second},\n)")
        );
    }

    #[test]
    fn a_broken_list_indents_relative_to_its_depth() {
        let first = "x".repeat(40);
        let second = "y".repeat(40);
        let items = vec![first.clone(), second.clone()];
        assert_eq!(
            render(&list(&items), 2),
            format!("        (\n            {first},\n            {second},\n        )")
        );
    }

    #[test]
    fn flat_width_counts_display_columns_not_bytes() {
        // Each CJK character is two columns wide. Forty of them overflow the
        // budget even though the byte length is not what decides.
        let wide = "名".repeat(40);
        let items = vec![wide.clone(), wide.clone()];
        let rendered = render(&list(&items), 0);
        assert!(rendered.starts_with("(\n"), "expected a broken list");
    }

    fn decorator_doc_for(source: &str) -> Doc {
        decorator_doc_and_text(source).0
    }

    // The decorator's doc and the flat text it must reproduce whenever it fits.
    fn decorator_doc_and_text(source: &str) -> (Doc, String) {
        let path = supported(b"fixtures/python/decorators/input.py");
        let tree = syntax::parse(source, &path).expect("decorated source parses");
        let renderer = Renderer::new(&path, source);
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == node::DECORATOR {
                return (renderer.decorator_doc(node), renderer.node_text(node));
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        panic!("source contains no decorator")
    }

    #[test]
    fn short_decorator_stays_flat_and_byte_identical() {
        assert_eq!(
            render(
                &decorator_doc_for("@app.route(\"/health\")\ndef f(): ...\n"),
                0
            ),
            "@app.route(\"/health\")"
        );
        assert_eq!(
            render(&decorator_doc_for("@property\ndef f(): ...\n"), 0),
            "@property"
        );
    }

    fn return_type_doc_for(source: &str) -> (Doc, String) {
        let path = supported("src/sample.py");
        let tree = syntax::parse(source, &path).expect("annotated source parses");
        let renderer = Renderer::new(&path, source);
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == node::FUNCTION_DEFINITION {
                let return_type = node
                    .child_by_field_name(field::RETURN_TYPE)
                    .expect("function has a return type");
                return (
                    renderer.type_doc(return_type),
                    renderer.node_text(return_type),
                );
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        panic!("source contains no function");
    }

    #[test]
    fn type_doc_flat_matches_node_text_for_short_types() {
        for (source, expected) in [
            ("def f() -> list[A | B]: ...\n", "list[A | B]"),
            ("def f() -> dict[str, int]: ...\n", "dict[str, int]"),
            ("def f() -> tuple[A, B]: ...\n", "tuple[A, B]"),
            ("def f() -> list[X]: ...\n", "list[X]"),
        ] {
            let (doc, flat) = return_type_doc_for(source);
            assert_eq!(flat, expected, "node_text for {source:?}");
            assert_eq!(render(&doc, 0), expected, "type_doc for {source:?}");
        }
    }

    #[test]
    fn type_doc_breaks_a_long_nested_subscript() {
        let (doc, _) = return_type_doc_for(
            "def f() -> dict[str, tuple[AutochargeInsuranceRemitsCron, PatientArAutochargeCron, OrganizationBillingProfile]]: ...\n",
        );
        assert_eq!(
            render(&doc, 0),
            concat!(
                "dict[\n",
                "    str,\n",
                "    tuple[\n",
                "        AutochargeInsuranceRemitsCron,\n",
                "        PatientArAutochargeCron,\n",
                "        OrganizationBillingProfile,\n",
                "    ],\n",
                "]",
            )
        );
    }

    #[test]
    fn long_decorator_breaks_one_argument_per_line_with_a_trailing_comma() {
        let doc = decorator_doc_for(
            "@app.get(\"/very/long/path/here\", response_model=VeryLongResponseModel, status_code=200)\ndef handler(): ...\n",
        );
        assert_eq!(
            render(&doc, 0),
            concat!(
                "@app.get(\n",
                "    \"/very/long/path/here\",\n",
                "    response_model=VeryLongResponseModel,\n",
                "    status_code=200,\n",
                ")",
            )
        );
    }

    #[test]
    fn a_decorator_argument_collection_breaks_one_element_per_line() {
        let doc = decorator_doc_for(concat!(
            "@pytest.mark.parametrize(\n",
            "    (\"readings\", \"expected\", \"why\"),\n",
            "    [\n",
            "        ([], WIDE, \"no completed runs\"),\n",
            "        ([3], WIDE, \"one narrow sample is not a streak\"),\n",
            "        ([3, 2], DEEP, \"two narrow samples in a row\"),\n",
            "        ([4, 4], DEEP, \"the threshold itself counts as narrow\"),\n",
            "        ([5, 2], WIDE, \"a later wide sample returns to the standard view\"),\n",
            "        ([2, 5], WIDE, \"the streak must be the two most recent\"),\n",
            "        ([2, 1, 9], DEEP, \"only the two most recent are read\"),\n",
            "    ],\n",
            ")\n",
            "def test_two_most_recent_widths_pick_deep(readings, expected, why): ...\n",
        ));
        assert_eq!(
            render(&doc, 0),
            concat!(
                "@pytest.mark.parametrize(\n",
                "    (\"readings\", \"expected\", \"why\"),\n",
                "    [\n",
                "        ([], WIDE, \"no completed runs\"),\n",
                "        ([3], WIDE, \"one narrow sample is not a streak\"),\n",
                "        ([3, 2], DEEP, \"two narrow samples in a row\"),\n",
                "        ([4, 4], DEEP, \"the threshold itself counts as narrow\"),\n",
                "        ([5, 2], WIDE, \"a later wide sample returns to the standard view\"),\n",
                "        ([2, 5], WIDE, \"the streak must be the two most recent\"),\n",
                "        ([2, 1, 9], DEEP, \"only the two most recent are read\"),\n",
                "    ],\n",
                ")",
            )
        );
    }

    #[test]
    fn a_nested_call_argument_breaks_at_its_own_column() {
        let doc = decorator_doc_for(
            "@outer(inner(\"aaaaaaaaaaaaaaaaaaaa\", \"bbbbbbbbbbbbbbbbbbbb\", \"cccccccccccccccccccc\"))\ndef f(): ...\n",
        );
        assert_eq!(
            render(&doc, 0),
            concat!(
                "@outer(\n",
                "    inner(\n",
                "        \"aaaaaaaaaaaaaaaaaaaa\",\n",
                "        \"bbbbbbbbbbbbbbbbbbbb\",\n",
                "        \"cccccccccccccccccccc\",\n",
                "    ),\n",
                ")",
            )
        );
    }

    // Every line a broken decorator produces, including the one its trailing
    // comma lands on, must fit the budget.
    #[test]
    fn a_broken_decorator_never_exceeds_the_line_width() {
        for (source, indent) in [
            (
                "@cache(**{\"key_one\": \"value_one\", \"key_two\": \"value_two\", \"key_three\": \"value\"})\ndef f(): ...\n",
                0,
            ),
            (
                "@app.get(\"/a/very/long/path/here\", methods=[\"GET\", \"POST\", \"PUT\", \"DELETE\"])\ndef f(): ...\n",
                0,
            ),
        ] {
            let doc = decorator_doc_for(source);
            for line in render(&doc, indent).lines() {
                assert!(
                    line.chars().count() <= LINE_WIDTH,
                    "line of {} columns: {line:?}",
                    line.chars().count()
                );
            }
        }
    }

    // The enclosing list's comma lands on the value's last line, so the value
    // is charged it and breaks rather than overflowing by one column.
    #[test]
    fn a_nested_collection_breaks_when_the_trailing_comma_would_overflow() {
        let doc = decorator_doc_for(concat!(
            "@cache(**{\"key_one\": \"value_one\", \"key_two\": \"value_two\", \"key_three\": \"value\"})\n",
            "def f(): ...\n",
        ));
        // At depth one the extra columns are what push it over.
        assert_eq!(
            render(&doc, 1),
            concat!(
                "    @cache(\n",
                "        **{\n",
                "            \"key_one\": \"value_one\",\n",
                "            \"key_two\": \"value_two\",\n",
                "            \"key_three\": \"value\",\n",
                "        },\n",
                "    )",
            )
        );
    }

    #[test]
    fn a_short_nested_decorator_argument_stays_flat_and_byte_identical() {
        for source in [
            "@app.get(\"/x\", meta={\"a\": 1, \"b\": 2})\ndef f(): ...\n",
            "@app.get(\"/x\", deps=[Depends(user), Depends(tenant)])\ndef f(): ...\n",
            "@app.get(\"/x\", handlers=(first, second))\ndef f(): ...\n",
            "@app.get(\"/x\", index=keys[1])\ndef f(): ...\n",
            "@app.get(\"/x\", nested=(inner(value)))\ndef f(): ...\n",
        ] {
            let (doc, text) = decorator_doc_and_text(source);
            assert_eq!(render(&doc, 0), text, "for {source:?}");
        }
    }

    // A dangling trailing comma is a formatting choice, so the projection
    // normalizes it away.
    #[test]
    fn a_dangling_trailing_comma_in_an_argument_is_normalized_away() {
        for (source, expected) in [
            ("@cache([1, 2,])\ndef f(): ...\n", "@cache([1, 2])"),
            ("@cache([1, 2])\ndef f(): ...\n", "@cache([1, 2])"),
            ("@cache({\"a\": 1,},)\ndef f(): ...\n", "@cache({\"a\": 1})"),
        ] {
            assert_eq!(
                render(&decorator_doc_for(source), 0),
                expected,
                "for {source:?}"
            );
        }
    }

    #[test]
    fn a_one_element_tuple_keeps_the_comma_that_makes_it_a_tuple() {
        assert_eq!(
            render(&decorator_doc_for("@deco((1,))\ndef f(): ...\n"), 0),
            "@deco((1,))"
        );
        assert_eq!(
            render(&decorator_doc_for("@deco((1, 2))\ndef f(): ...\n"), 0),
            "@deco((1, 2))"
        );
    }

    // A slice's colon has no place in a comma-separated rebuild.
    #[test]
    fn a_sliced_subscript_argument_stays_flat() {
        for (source, expected) in [
            ("@deco(x[1:2])\ndef f(): ...\n", "@deco(x[1: 2])"),
            ("@deco(x[1:2, 3])\ndef f(): ...\n", "@deco(x[1: 2, 3])"),
        ] {
            assert_eq!(
                render(&decorator_doc_for(source), 0),
                expected,
                "for {source:?}"
            );
        }
    }

    #[test]
    fn a_long_subscript_index_breaks_one_element_per_line() {
        let doc = decorator_doc_for(concat!(
            "@deco(x[\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\", \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\", \"cc\"])\n",
            "def f(): ...\n",
        ));
        assert_eq!(
            render(&doc, 0),
            concat!(
                "@deco(\n",
                "    x[\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\", \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\", \"cc\"],\n",
                ")",
            )
        );
    }

    // `@deco(x for x in y)` satisfies the call's `arguments` field but has no
    // comma-separated items, so breaking it would emit invalid Python.
    #[test]
    fn a_generator_expression_argument_stays_flat() {
        for source in [
            "@deco(x for x in y)\ndef f(): ...\n",
            "@deco(x for x in some_really_long_iterable_name_that_overflows_the_budget)\ndef f(): ...\n",
        ] {
            let doc = decorator_doc_for(source);
            assert_eq!(
                render(&doc, 0),
                source.lines().next().expect("a decorator line"),
                "for {source:?}"
            );
        }
    }

    fn parameter_doc_for(source: &str) -> Doc {
        let path = supported("src/sample.py");
        let tree = syntax::parse(source, &path).expect("parameterized source parses");
        let renderer = Renderer::new(&path, source);
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == node::PARAMETERS {
                let mut cursor = node.walk();
                let first = node
                    .named_children(&mut cursor)
                    .next()
                    .expect("the function has a parameter");
                return renderer.parameter_doc(first);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        panic!("source contains no parameter list");
    }

    #[test]
    fn a_short_parameter_is_flat_and_byte_identical_to_node_text() {
        for (source, expected) in [
            ("def f(a: int) -> None: ...\n", "a: int"),
            ("def f(a: int = 1) -> None: ...\n", "a: int = 1"),
            ("def f(a=1) -> None: ...\n", "a=1"),
            ("def f(a: dict = {}) -> None: ...\n", "a: dict = {}"),
            ("def f(a=[1, 2]) -> None: ...\n", "a=[1, 2]"),
            (
                "def f(*args: int, **kwargs: str) -> None: ...\n",
                "*args: int",
            ),
        ] {
            assert_eq!(
                render(&parameter_doc_for(source), 0),
                expected,
                "for {source:?}"
            );
        }
    }

    #[test]
    fn a_long_default_value_breaks_once_the_parameter_list_breaks() {
        let doc = parameter_doc_for(concat!(
            "def f(\n",
            "    routes: dict = {\"primary\": \"/a/very/long/path\", \"fallback\": \"/another/long/path\"},\n",
            ") -> None: ...\n",
        ));
        assert_eq!(
            render(&doc, 1),
            concat!(
                "    routes: dict = {\n",
                "        \"primary\": \"/a/very/long/path\",\n",
                "        \"fallback\": \"/another/long/path\",\n",
                "    }",
            )
        );
    }
}
