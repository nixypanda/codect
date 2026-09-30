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

/// One leaf token with explicit glue flags. `glue_before` and `glue_after`
/// suppress the default space on that side, which is how `keyword=value` is
/// written without spaces while `name: T = default` keeps them.
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

    /// The path and its derived language, for building a [`ProjectionError`].
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

    /// The first child of `kind`, named or not. Python needs this for anonymous
    /// keyword tokens such as `async`.
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

    /// Canonical text for a type, parameter list, or expression fragment.
    pub(crate) fn node_text(&self, node: Node<'_>) -> String {
        let mut tokens = Vec::new();
        self.push_tokens(node, &mut tokens);
        join(&tokens)
    }

    /// A decorator, with a breakable argument list when its expression is a
    /// call. A bare decorator (`@property`) or any other expression stays flat.
    ///
    /// The `decorator` node has no fields; its first named child is the
    /// expression. When that expression is a `call`, the call's `arguments`
    /// (`argument_list`) named children are the items, rendered through
    /// [`Renderer::bracket_list`] so a long decorator breaks one argument per
    /// indented line with a trailing comma while a short one stays inline.
    pub(crate) fn decorator_doc(&self, node: Node<'_>) -> Doc {
        let mut cursor = node.walk();
        let expression = node.named_children(&mut cursor).next();
        if let Some(call) = expression.filter(|child| child.kind() == node::CALL)
            && let (Some(function), Some(arguments)) = (
                call.child_by_field_name("function"),
                call.child_by_field_name("arguments"),
            )
        {
            return Doc::Group(Box::new(Doc::Concat(vec![
                Doc::Text("@".to_owned()),
                Doc::Text(self.node_text(function)),
                self.node_bracket_list(arguments, "(", ")"),
            ])));
        }
        Doc::Text(self.node_text(node))
    }

    /// A bracketed list body: inline while the enclosing [`Doc::Group`] fits,
    /// one item per indented line when it breaks.
    ///
    /// `open`/`close` are the delimiters (`()` for parameters, `[]` for type
    /// parameters). The trailing comma is emitted only when the group breaks, so
    /// appending an item changes exactly one line. This is deliberately ungrouped
    /// so the caller can group the whole declaration header, letting the fit
    /// check see the return type and other trailing text.
    ///
    /// Items are [`Doc`]s so a nested list (a subscript inside a type, or a long
    /// parameter type) re-decides at its own column after the outer list breaks.
    pub(crate) fn bracket_list(&self, items: Vec<Doc>, open: &str, close: &str) -> Doc {
        if items.is_empty() {
            return Doc::Text(format!("{open}{close}"));
        }

        let mut inner: Vec<Doc> = vec![Doc::SoftNil];
        for (index, item) in items.into_iter().enumerate() {
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

    /// [`Renderer::bracket_list`] over a node's named children rendered as flat
    /// text. Used for lists whose items have no internal break points (call
    /// arguments, superclasses, type parameters).
    pub(crate) fn node_bracket_list(&self, node: Node<'_>, open: &str, close: &str) -> Doc {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(Doc::Text(self.node_text(child)));
        }
        self.bracket_list(items, open, close)
    }

    /// A function's parameter list, with each parameter rendered by
    /// [`Renderer::parameter_doc`] so a long parameter type can break once the
    /// parameter list itself breaks.
    pub(crate) fn parameters_doc(&self, node: Node<'_>) -> Doc {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(self.parameter_doc(child));
        }
        self.bracket_list(items, "(", ")")
    }

    /// A parameter, keeping its declared type as a nested [`Doc`].
    ///
    /// Only a `typed_parameter` or `typed_default_parameter` (one with a `type`
    /// field) is split; every other parameter stays flat. The split renders the
    /// tokens before and after the type node separately, re-inserting the exact
    /// boundary space, so the flat form is byte-identical to
    /// [`Renderer::node_text`].
    fn parameter_doc(&self, node: Node<'_>) -> Doc {
        let Some(type_node) = node.child_by_field_name(field::TYPE) else {
            return Doc::Text(self.node_text(node));
        };

        let type_id = type_node.id();
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut seen_type = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.id() == type_id {
                seen_type = true;
            } else if seen_type {
                self.push_tokens(child, &mut after);
            } else {
                self.push_tokens(child, &mut before);
            }
        }

        let mut type_tokens = Vec::new();
        self.push_tokens(type_node, &mut type_tokens);
        if type_tokens.is_empty() {
            return Doc::Text(self.node_text(node));
        }

        let mut before_text = join(&before);
        if let (Some(left), Some(first)) = (before.last(), type_tokens.first())
            && !left.glue_after
            && !first.glue_before
            && needs_space(&left.text, &first.text)
        {
            before_text.push(' ');
        }

        let mut after_text = join(&after);
        if let (Some(last), Some(right)) = (type_tokens.last(), after.first())
            && !last.glue_after
            && !right.glue_before
            && needs_space(&last.text, &right.text)
        {
            after_text.insert(0, ' ');
        }

        Doc::Concat(vec![
            Doc::Text(before_text),
            self.type_doc(type_node),
            Doc::Text(after_text),
        ])
    }

    /// A recursive type [`Doc`].
    ///
    /// Only the node kinds that appear in Python type positions are handled:
    /// the `type` wrapper, `generic_type` subscripts, and `binary_operator`
    /// unions. Every other kind falls back to flat text, so an unmodeled type
    /// cannot regress and always renders byte-identically to
    /// [`Renderer::node_text`].
    ///
    /// This grammar (tree-sitter-python 0.25) represents annotations with a
    /// `type` wrapper around a `generic_type` (`list[int]`) rather than a
    /// `subscript`; a bare `subscript` in an expression position is left flat.
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

    /// A subscript type such as `dict[str, int]` or `tuple[A, B]`.
    ///
    /// The value (`dict`, `tuple`, or a dotted attribute) stays flat and each
    /// bracketed argument group becomes a breakable list whose items are
    /// recursively typed. The whole node is grouped so it re-decides at its own
    /// column after an enclosing parameter list breaks.
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

    /// A union such as `A | B | C`, breaking before each `|`. Any other operator
    /// (or a non-union shape) stays flat.
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

    /// Flattens a `|` chain (left- or right-leaning) into its operands.
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

/// Spacing is a function of the two adjacent leaf tokens, never of source
/// whitespace.
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
    use base::RepoPath;

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

    /// The document shape `bracket_list_doc` builds, for width tests.
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
        let path = supported(b"fixtures/python/decorators/input.py");
        let tree = syntax::parse(source, &path).expect("decorated source parses");
        let renderer = Renderer::new(&path, source);
        let mut stack = vec![tree.root_node()];
        let mut decorator = None;
        while let Some(node) = stack.pop() {
            if node.kind() == node::DECORATOR {
                decorator = Some(node);
                break;
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        renderer.decorator_doc(decorator.expect("source contains a decorator"))
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

    /// The `type_doc` for the return annotation of the first function in
    /// `source`, alongside its flat `node_text`.
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
}
