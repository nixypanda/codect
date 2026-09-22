//! Canonical rendering of Python declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs. Declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks; source slices are used only for atomic literals such as strings,
//! where internal bytes carry meaning. Terminal width is never consulted.

use ownai_core::{Language, ProjectionError, RepoPath};
use tree_sitter::Node;

use crate::syntax::{
    self, COMMENT, DEFAULT_PARAMETER, FIELD_NAME, FIELD_VALUE, KEYWORD_ARGUMENT, STRING,
};

/// A small layout document. `Indent` is relative, so the same document can be
/// rendered at any nesting depth.
#[derive(Clone, Debug)]
pub(crate) enum Doc {
    Text(String),
    Line,
    Indent(Box<Doc>),
    Concat(Vec<Doc>),
}

struct Output {
    text: String,
    depth: usize,
    at_line_start: bool,
}

impl Output {
    fn write(&mut self, value: &str) {
        if self.at_line_start {
            for _ in 0..self.depth {
                self.text.push_str("    ");
            }
            self.at_line_start = false;
        }
        self.text.push_str(value);
    }

    fn line(&mut self) {
        self.text.push('\n');
        self.at_line_start = true;
    }
}

/// Renders a document at a nesting depth of four spaces per level.
pub(crate) fn render(doc: &Doc, depth: usize) -> String {
    let mut output = Output {
        text: String::new(),
        depth,
        at_line_start: true,
    };
    render_into(doc, &mut output);
    output.text
}

fn render_into(doc: &Doc, output: &mut Output) {
    match doc {
        Doc::Text(value) => output.write(value),
        Doc::Line => output.line(),
        Doc::Indent(inner) => {
            output.depth += 1;
            render_into(inner, output);
            output.depth -= 1;
        }
        Doc::Concat(parts) => {
            for part in parts {
                render_into(part, output);
            }
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

    /// The comma-separated named children of an `argument_list`, without the
    /// surrounding parentheses. Keyword arguments render as `name=value`.
    pub(crate) fn argument_list_text(&self, node: Node<'_>) -> String {
        let mut parts = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            parts.push(self.node_text(child));
        }
        parts.join(", ")
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
}
