//! Canonical rendering of Python declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs. Declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks; source slices are used only for atomic literals such as strings,
//! where internal bytes carry meaning. Terminal width is never consulted.

use ownai_core::{LINE_WIDTH, Language, ProjectionError, RepoPath};
use tree_sitter::Node;
use unicode_width::UnicodeWidthStr;

use crate::syntax::{
    self, COMMENT, DEFAULT_PARAMETER, FIELD_NAME, FIELD_VALUE, KEYWORD_ARGUMENT, STRING,
};

/// A small layout document. `Indent` is relative, so the same document can be
/// rendered at any nesting depth.
///
/// [`Doc::Group`] is the only width-sensitive construct: it renders flat (soft
/// breaks as spaces) when the flat form fits within [`LINE_WIDTH`], and broken
/// (soft breaks as newlines) otherwise. Hard [`Doc::Line`] breaks are always
/// newlines and force any enclosing group to break.
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
    path: &'a RepoPath,
    source: &'a str,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(path: &'a RepoPath, source: &'a str) -> Self {
        Self { path, source }
    }

    pub(crate) fn invariant<T>(
        &self,
        node: Node<'_>,
        detail: impl Into<String>,
    ) -> Result<T, ProjectionError> {
        Err(ProjectionError::AstInvariant {
            path: self.path.clone(),
            language: Language::Python,
            range: syntax::node_span(node),
            detail: detail.into(),
        })
    }

    pub(crate) fn path(&self) -> &RepoPath {
        self.path
    }

    pub(crate) fn slice(&self, node: Node<'_>) -> &str {
        self.source.get(node.byte_range()).unwrap_or("")
    }

    pub(crate) fn field_text(&self, node: Node<'_>, field: &str) -> Option<&str> {
        node.child_by_field_name(field)
            .and_then(|child| self.source.get(child.byte_range()))
    }

    fn push_tokens(&self, node: Node<'_>, out: &mut Vec<Tok>) {
        let kind = node.kind();
        if kind == COMMENT {
            return;
        }
        if kind == STRING {
            out.push(Tok::plain(self.slice(node)));
            return;
        }
        if kind == KEYWORD_ARGUMENT {
            if let Some(name) = node.child_by_field_name(syntax::FIELD_NAME) {
                self.push_tokens(name, out);
            }
            out.push(Tok::glued("="));
            if let Some(value) = node.child_by_field_name(syntax::FIELD_VALUE) {
                self.push_tokens(value, out);
            }
            return;
        }
        // An unannotated default is written `name=value`; an annotated default
        // is written `name: T = value` and is handled by the generic path.
        if kind == DEFAULT_PARAMETER {
            if let Some(name) = node.child_by_field_name(FIELD_NAME) {
                self.push_tokens(name, out);
            }
            out.push(Tok::glued("="));
            if let Some(value) = node.child_by_field_name(FIELD_VALUE) {
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

    pub(crate) fn child_of_kind<'t>(&self, node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|child| child.kind() == kind)
    }

    pub(crate) fn has_child_of_kind(&self, node: Node<'_>, kind: &str) -> bool {
        self.child_of_kind(node, kind).is_some()
    }

    /// A bracketed list body: inline while the enclosing [`Doc::Group`] fits,
    /// one item per indented line when it breaks.
    ///
    /// `open`/`close` are the delimiters (`()` for parameters, `[]` for type
    /// parameters). The trailing comma is emitted only when the group breaks, so
    /// appending an item changes exactly one line. This is deliberately ungrouped
    /// so the caller can group the whole declaration header, letting the fit
    /// check see the return type and other trailing text.
    pub(crate) fn bracket_list(&self, node: Node<'_>, open: &str, close: &str) -> Doc {
        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            items.push(self.node_text(child));
        }
        if items.is_empty() {
            return Doc::Text(format!("{open}{close}"));
        }

        let mut inner: Vec<Doc> = vec![Doc::SoftNil];
        for (index, item) in items.into_iter().enumerate() {
            if index > 0 {
                inner.push(Doc::Text(",".to_owned()));
                inner.push(Doc::SoftLine);
            }
            inner.push(Doc::Text(item));
        }
        inner.push(Doc::Broken(","));

        Doc::Concat(vec![
            Doc::Text(open.to_owned()),
            Doc::Indent(Box::new(Doc::Concat(inner))),
            Doc::SoftNil,
            Doc::Text(close.to_owned()),
        ])
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
}
