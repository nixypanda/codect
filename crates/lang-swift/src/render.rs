//! Canonical rendering of Swift declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs. Node text is rebuilt
//! from leaf tokens with fixed spacing rules and fixed structural line breaks;
//! executable bodies, stored-property initializers, macro definitions, and any
//! closure nested in an attribute argument, default value, or interpolation are
//! replaced by a frozen `{ ... }` placeholder. Terminal width is never
//! consulted; all width-sensitive wrapping goes through [`Doc`].

use std::ops::Range;

use base::SupportedPath;
pub(crate) use base::doc::{Doc, render};
use tree_sitter::Node;

use crate::syntax::{field, is_body, node};

/// Split free text into lexical tokens, merging the multi-character operators
/// Swift uses in conditions and signatures. Used to normalize guard headers so
/// `os(iOS)` and `os ( iOS )` produce identical canonical text and keys.
pub(crate) fn lex(text: &str) -> Vec<String> {
    const OPERATOR_CHARS: &str = "=!<>&|+-*/?:.,()[]{}";

    let mut out = Vec::new();
    let mut word = String::new();
    let mut operator = String::new();

    let flush_word = |out: &mut Vec<String>, word: &mut String| {
        if !word.is_empty() {
            out.push(std::mem::take(word));
        }
    };
    let flush_operator = |out: &mut Vec<String>, operator: &mut String| {
        if !operator.is_empty() {
            out.push(std::mem::take(operator));
        }
    };

    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            flush_operator(&mut out, &mut operator);
            word.push(character);
        } else if character.is_whitespace() {
            flush_word(&mut out, &mut word);
            flush_operator(&mut out, &mut operator);
        } else if OPERATOR_CHARS.contains(character) {
            flush_word(&mut out, &mut word);
            operator.push(character);
        } else {
            // A Unicode punctuation/symbol character not in the ASCII operator
            // set (e.g. an operator declaration such as `???`): emit it whole.
            flush_word(&mut out, &mut word);
            flush_operator(&mut out, &mut operator);
            out.push(character.to_string());
        }
    }
    flush_word(&mut out, &mut word);
    flush_operator(&mut out, &mut operator);
    out
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "if" | "else"
            | "elseif"
            | "while"
            | "for"
            | "in"
            | "guard"
            | "switch"
            | "case"
            | "default"
            | "where"
            | "return"
            | "yield"
            | "as"
            | "is"
            | "try"
            | "await"
            | "throw"
            | "repeat"
            | "each"
            | "some"
            | "any"
            | "inout"
            | "borrowing"
            | "consuming"
            | "static"
            | "class"
            | "mutating"
            | "nonmutating"
            | "nonisolated"
            | "async"
            | "throws"
            | "rethrows"
    )
}

fn is_spaced_operator(token: &str) -> bool {
    matches!(
        token,
        "==" | "!=" | "===" | "!==" | "&&" | "||" | ">=" | "<=" | "->" | "..." | "..<"
    )
}

/// Canonical spacing between adjacent Swift tokens.
///
/// The rules are deliberately token-based (never byte-offset based) so that
/// original whitespace never leaks into `canonical_text`. Literal-text tokens
/// that already carry whitespace (string segments, raw text) are never padded.
fn needs_space(previous: &str, next: &str) -> bool {
    if previous.is_empty() || next.is_empty() {
        return false;
    }
    if previous.ends_with(char::is_whitespace) || next.starts_with(char::is_whitespace) {
        return false;
    }

    // Tokens that always attach to what precedes them. Generic `<`/`>` attach
    // here; a comparison operator is marked with its own spaces by the
    // grammar-aware leaf walker, so no token-level guess is needed.
    if matches!(
        next,
        ")" | "]" | "}" | ">" | "," | ";" | "." | ":" | "?" | "!" | "\""
    ) {
        return false;
    }

    // An opening delimiter is spaced only after a keyword, separator, or infix
    // operator. Otherwise it attaches (`foo(`, `@escaping (` is spaced because
    // `@escaping` is treated as a modifier keyword below).
    if matches!(next, "(" | "[" | "<") {
        return is_keyword(previous)
            || previous.starts_with('@')
            || matches!(
                previous,
                "," | ";" | ":" | "=" | "->" | "&&" | "||" | "+" | "-" | "*" | "/" | "<"
            );
    }

    // Prefix punctuation attaches to what follows it.
    if matches!(
        previous,
        "(" | "[" | "{" | "<" | "." | "@" | "#" | "?" | "!" | "\"" | "\\(" | "\\" | "/"
    ) {
        return false;
    }

    // Separators and binary operators are surrounded by one space.
    if matches!(previous, "," | ";" | ":" | "->" | "=" | "&") || is_spaced_operator(previous) {
        return true;
    }
    if matches!(next, "=" | "&") || is_spaced_operator(next) {
        return true;
    }

    true
}

/// Join leaf tokens with canonical spacing, preserving any whitespace already
/// embedded in literal-text tokens.
pub(crate) fn join(tokens: &[String]) -> String {
    let mut text = String::new();
    let mut previous: Option<&str> = None;
    for token in tokens {
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

pub(crate) fn is_comment(kind: &str) -> bool {
    matches!(kind, node::COMMENT | node::MULTILINE_COMMENT)
}

/// String and regex literals keep their written text (delimiters, escapes,
/// interpolation punctuation, comment-like text) except for executable closures
/// nested in interpolation, which are replaced by the `{ ... }` placeholder.
fn is_literal(kind: &str) -> bool {
    matches!(
        kind,
        "line_string_literal"
            | "raw_string_literal"
            | "multi_line_string_literal"
            | "regex_literal"
    )
}

pub(crate) struct Renderer<'a> {
    _path: &'a SupportedPath,
    source: &'a str,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(path: &'a SupportedPath, source: &'a str) -> Self {
        Self {
            _path: path,
            source,
        }
    }

    pub(crate) fn slice(&self, node: Node<'_>) -> &'a str {
        self.source.get(node.byte_range()).unwrap_or("")
    }

    fn text_of(&self, node: Node<'_>) -> &str {
        self.source.get(node.byte_range()).unwrap_or("")
    }

    /// Leaf token text with grammar-driven spacing. `<`/`>` directly under a
    /// `comparison_expression` and `?`/`:` directly under a `ternary_expression`
    /// get their own spaces. Optional-type `?` and annotation `:` have other
    /// parents and keep their attached spelling. No token-only heuristic.
    fn leaf_token(&self, node: Node<'_>) -> String {
        let text = self.text_of(node);
        let parent_kind = node.parent().map(|parent| parent.kind());
        if (matches!(text, "<" | ">") && parent_kind == Some("comparison_expression"))
            || (matches!(text, "?" | ":") && parent_kind == Some("ternary_expression"))
        {
            return format!(" {text} ");
        }
        text.to_owned()
    }

    /// Leaf-token walk. When `strip_closures` is true every `lambda_literal`
    /// anywhere in the subtree (including nested inside interpolation) collapses
    /// to the `{ ... }` placeholder, and no statement from any depth leaks out.
    fn push_tokens(&self, node: Node<'_>, out: &mut Vec<String>, strip_closures: bool) {
        let kind = node.kind();
        if is_comment(kind) {
            return;
        }
        if strip_closures && kind == node::LAMBDA_LITERAL {
            out.push("{ ... }".to_owned());
            return;
        }
        if is_literal(kind) {
            out.push(self.literal_text(node));
            return;
        }
        if node.child_count() == 0 {
            let text = self.leaf_token(node);
            if !text.is_empty() {
                out.push(text);
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.push_tokens(child, out, strip_closures);
        }
    }

    /// Preserve a literal's written text and delimiters. Outside interpolation
    /// every child is copied verbatim, so comment-looking literal text and
    /// original whitespace are never reinterpreted. Inside interpolation the
    /// executable expression is token-canonicalized (comments and incidental
    /// whitespace dropped, nested closures stripped at every depth).
    fn literal_text(&self, node: Node<'_>) -> String {
        match node.kind() {
            "line_string_literal" | "multi_line_string_literal" => {
                let mut out = String::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == node::INTERPOLATED_EXPRESSION {
                        out.push_str(&self.interpolation_text(child));
                    } else if child.kind() == node::LAMBDA_LITERAL {
                        out.push_str("{ ... }");
                    } else if is_comment(child.kind()) {
                        // A real comment node inside a literal is not literal
                        // text; comment-looking text lives in `*_str_text`.
                        continue;
                    } else {
                        out.push_str(self.text_of(child));
                    }
                }
                out
            }
            "raw_string_literal" => {
                // Raw strings do have `raw_str_interpolation` children (`\#(...)`
                // with one or more hashes). Fixed text and delimiters are copied
                // verbatim; only the executable interpolation is canonicalized.
                let mut out = String::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "raw_str_interpolation" {
                        out.push_str(&self.raw_interpolation_text(child));
                    } else if is_comment(child.kind()) {
                        continue;
                    } else {
                        out.push_str(self.text_of(child));
                    }
                }
                out
            }
            // Regex literals are opaque in the pinned grammar; keep their exact
            // text and splice over any lambda defensively.
            _ => self.raw_literal_text(node),
        }
    }

    /// Canonicalize the executable part of a raw-string interpolation while
    /// keeping the `#`-delimited opener (`\#(`, `\##(`, …) and any punctuation
    /// exactly as written.
    fn raw_interpolation_text(&self, node: Node<'_>) -> String {
        let mut out = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                node::INTERPOLATED_EXPRESSION => out.push_str(&self.interpolation_text(child)),
                "raw_str_interpolation_start" => out.push_str(self.text_of(child)),
                _ if is_comment(child.kind()) => {}
                _ => out.push_str(self.text_of(child)),
            }
        }
        out
    }

    /// Verbatim literal text with every nested lambda replaced by `{ ... }`.
    fn raw_literal_text(&self, node: Node<'_>) -> String {
        let raw = self.text_of(node);
        let mut ranges = Vec::new();
        collect_lambdas(node, &mut ranges);
        if ranges.is_empty() {
            return raw.to_owned();
        }
        ranges.sort_unstable_by_key(|(start, _)| *start);
        let mut out = String::new();
        let mut cursor = 0usize;
        for (start, end) in ranges {
            let from = start.saturating_sub(node.start_byte());
            let to = end.saturating_sub(node.start_byte());
            if from >= cursor && from <= raw.len() {
                out.push_str(&raw[cursor..from]);
                out.push_str("{ ... }");
                cursor = to.min(raw.len());
            }
        }
        if cursor < raw.len() {
            out.push_str(&raw[cursor..]);
        }
        out
    }

    /// Canonical text of one `\(...)` interpolation: the interpolation
    /// punctuation is kept, the executable expression is rebuilt from filtered
    /// tokens, and closures at any depth become `{ ... }`. Comment-looking
    /// literal text is never treated as a comment here; only grammar comment
    /// nodes inside the executable expression are dropped.
    fn interpolation_text(&self, node: Node<'_>) -> String {
        let mut tokens = Vec::new();
        let mut saw_value = false;
        // Collect (child, field) pairs first: a tree cursor cannot be queried
        // for its field name while it is mutably borrowed by `children`.
        let children: Vec<_> = {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .enumerate()
                .map(|(index, child)| (child, node.field_name_for_child(index as u32)))
                .collect()
        };
        for (child, child_field) in children {
            if child_field == Some(field::VALUE) {
                self.push_tokens(child, &mut tokens, true);
                saw_value = true;
                continue;
            }
            if is_comment(child.kind()) {
                continue;
            }
            let text = self.text_of(child);
            if child.child_count() == 0 {
                if !text.trim().is_empty() {
                    // Interpolation punctuation such as `\(` and `)`.
                    tokens.push(text.to_owned());
                }
            } else {
                // Named structure such as an interpolation label.
                self.push_tokens(child, &mut tokens, true);
            }
        }
        if saw_value {
            join(&tokens)
        } else {
            self.text_of(node).to_owned()
        }
    }

    fn tokens(&self, node: Node<'_>) -> Vec<String> {
        let mut out = Vec::new();
        self.push_tokens(node, &mut out, false);
        out
    }

    fn stripped_tokens(&self, node: Node<'_>) -> Vec<String> {
        let mut out = Vec::new();
        self.push_tokens(node, &mut out, true);
        out
    }

    pub(crate) fn node_text(&self, node: Node<'_>) -> String {
        join(&self.tokens(node))
    }

    /// Text of a node that may contain executable closures; they become
    /// `{ ... }` placeholders at every depth.
    pub(crate) fn stripped_text(&self, node: Node<'_>) -> String {
        join(&self.stripped_tokens(node))
    }

    /// Structural canonicalizer for a retained type, expression, or attribute
    /// argument. Bracketed sublists become independent `Doc::Group`s, so a long
    /// nested list breaks within the shared 80-column budget instead of being
    /// flattened into an unbreakable `Doc::Text`. Comments and closure bodies
    /// never appear.
    pub(crate) fn node_doc(&self, node: Node<'_>) -> Doc {
        let mut elems = Vec::new();
        self.push_struct_elems(node, &mut elems, true);
        elements_doc(&group_token_runs(elems))
    }

    /// Grammar-aware flattening. Only genuine `type_arguments` nodes produce
    /// `<...>` groups; a `comparison_expression`'s `<`/`>` remain plain tokens,
    /// so valid boolean defaults and attribute arguments are never mistaken
    /// for generic lists.
    fn push_struct_elems(&self, node: Node<'_>, out: &mut Vec<Elem>, strip_closures: bool) {
        let kind = node.kind();
        if is_comment(kind) {
            return;
        }
        if strip_closures && kind == node::LAMBDA_LITERAL {
            out.push(Elem::Token("{ ... }".to_owned()));
            return;
        }
        if is_literal(kind) {
            out.push(Elem::Token(self.literal_text(node)));
            return;
        }
        if kind == "type_arguments" {
            out.push(Elem::List {
                items: self.type_argument_items(node),
                open: "<",
                close: ">",
            });
            return;
        }
        if node.child_count() == 0 {
            let text = self.leaf_token(node);
            if !text.is_empty() {
                out.push(Elem::Token(text));
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.push_struct_elems(child, out, strip_closures);
        }
    }

    /// One doc per named type child of a `type_arguments` node. Source commas
    /// are not copied, so trailing commas cannot become phantom items, while
    /// nested `type_arguments` stay independently breakable groups.
    fn type_argument_items(&self, node: Node<'_>) -> Vec<Doc> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if is_comment(child.kind()) {
                continue;
            }
            let mut elems = Vec::new();
            self.push_struct_elems(child, &mut elems, true);
            if elems.is_empty() {
                continue;
            }
            out.push(elements_doc(&group_token_runs(elems)));
        }
        out
    }

    /// The `attribute` children of a declaration's `modifiers`, in source order.
    pub(crate) fn attributes<'tree>(&self, node: Node<'tree>) -> Vec<Node<'tree>> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == node::MODIFIERS {
                let mut mcursor = child.walk();
                for modifier in child.children(&mut mcursor) {
                    if modifier.kind() == node::ATTRIBUTE {
                        out.push(modifier);
                    }
                }
            }
        }
        out
    }

    /// Non-attribute modifier tokens of a declaration (access control, `static`,
    /// `final`, isolation modifiers, and so on).
    fn modifier_elems(&self, node: Node<'_>) -> Vec<Elem> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == node::MODIFIERS {
                let mut mcursor = child.walk();
                for modifier in child.children(&mut mcursor) {
                    if modifier.kind() != node::ATTRIBUTE && !is_comment(modifier.kind()) {
                        for token in self.tokens(modifier) {
                            out.push(Elem::Token(token));
                        }
                    }
                }
            }
        }
        out
    }

    /// Build the interface Doc of a declaration: every direct child before its
    /// body, with parameter lists and generic parameter lists as `Doc::Group`s.
    /// For stored properties and globals the `=` that begins an initializer is
    /// also a stopping point, so initializers never leak into canonical text.
    pub(crate) fn interface(&self, node: Node<'_>, stop_at_equal: bool) -> Doc {
        let mut elems = self.modifier_elems(node);
        let children: Vec<Node<'_>> = {
            let mut cursor = node.walk();
            node.children(&mut cursor).collect()
        };

        let mut index = 0usize;
        while index < children.len() {
            let child = children[index];
            let kind = child.kind();
            if is_body(kind) || (stop_at_equal && self.text_of(child) == "=") {
                break;
            }
            if kind == node::MODIFIERS {
                index += 1;
                continue;
            }
            if self.text_of(child) == "(" {
                // Parameter list: gather direct children up to the matching `)`.
                let mut end = index;
                let mut depth = 0i32;
                for (offset, candidate) in children.iter().enumerate().skip(index) {
                    match self.text_of(*candidate) {
                        "(" => depth += 1,
                        ")" => {
                            depth -= 1;
                            if depth == 0 {
                                end = offset;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let items = self.parameter_items(node, (index + 1)..end);
                elems.push(Elem::List {
                    items,
                    open: "(",
                    close: ")",
                });
                index = end + 1;
                continue;
            }
            if kind == node::TYPE_PARAMETERS {
                let items = self.generic_items(child);
                elems.push(Elem::List {
                    items,
                    open: "<",
                    close: ">",
                });
                index += 1;
                continue;
            }
            if kind == node::THROWS_CLAUSE {
                // The `throws` keyword is part of the clause span but not a
                // child token; re-emit it attached to the typed error list.
                let parts = self.tokens(child);
                elems.push(Elem::Token(format!("throws{}", join(&parts))));
                index += 1;
                continue;
            }
            // Types and other header fragments can themselves contain long
            // bracketed lists; keep them structural instead of flattening.
            let tokens = self.stripped_tokens(child);
            if let (Some(first), Some(last)) = (tokens.first(), tokens.last()) {
                elems.push(Elem::Doc {
                    doc: self.node_doc(child),
                    first: first.clone(),
                    last: last.clone(),
                });
            }
            index += 1;
        }

        elements_doc(&elems)
    }

    /// One doc per `type_parameter` (or `where` constraint) child. Comment
    /// nodes are named children of `type_parameters`; treating them as items
    /// produces phantom commas such as `<T, , U>`, so they are filtered out.
    fn generic_items(&self, node: Node<'_>) -> Vec<Doc> {
        let mut out = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if is_comment(child.kind()) {
                continue;
            }
            out.push(self.node_doc(child));
        }
        out
    }

    /// Build one parameter doc per `parameter` child. Defaults come from the
    /// grammar's `default_value` field rather than from raw `=` / expression
    /// siblings, so comments between the parameter and its default cannot make
    /// the default disappear or shift onto the wrong parameter.
    fn parameter_items(&self, parent: Node<'_>, range: Range<usize>) -> Vec<Doc> {
        let mut out = Vec::new();
        for index in range.clone() {
            let Some(child) = parent.child(index as u32) else {
                continue;
            };
            if child.kind() != node::PARAMETER {
                continue;
            }
            let mut parts = vec![self.node_doc(child)];
            let mut cursor = index + 1;
            while cursor < range.end {
                let Some(next) = parent.child(cursor as u32) else {
                    break;
                };
                if next.kind() == node::PARAMETER || self.text_of(next) == "," {
                    break;
                }
                if parent.field_name_for_child(cursor as u32) == Some(field::DEFAULT_VALUE) {
                    parts.push(Doc::Text(" = ".to_owned()));
                    parts.push(self.node_doc(next));
                }
                cursor += 1;
            }
            out.push(Doc::Concat(parts));
        }
        out
    }

    /// Normalize a directive node into a `(kind_word, header, guard)` triple.
    /// `header` is the canonical Swift form (`#if os(iOS)`); `guard` is the
    /// key fragment (`if:os(iOS)`). Whitespace and comments are ignored and
    /// punctuation is canonicalized so spelling variants collapse together.
    pub(crate) fn directive(&self, node: Node<'_>) -> (String, String, String) {
        let mut tokens = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if is_comment(child.kind()) {
                continue;
            }
            if child.child_count() == 0 {
                tokens.extend(lex(self.text_of(child)));
            } else {
                let mut inner = Vec::new();
                self.push_tokens(child, &mut inner, false);
                for token in inner {
                    tokens.extend(lex(&token));
                }
            }
        }
        if tokens.first().is_some_and(|token| token == "#") {
            tokens.remove(0);
        }
        if tokens.is_empty() {
            return ("else".to_owned(), "#else".to_owned(), "else".to_owned());
        }
        let keyword = tokens[0].trim_start_matches('#').to_owned();
        let condition = join(&tokens[1..]);
        let header = if condition.is_empty() {
            format!("#{keyword}")
        } else {
            format!("#{keyword} {condition}")
        };
        let guard = if condition.is_empty() {
            keyword.clone()
        } else {
            format!("{keyword}:{condition}")
        };
        (keyword, header, guard)
    }

    /// The canonical text of a single accessor specifier (a `getter_specifier`,
    /// `setter_specifier`, `modify_specifier`, or observer clause node):
    /// `[mutating|nonmutating] get|set|_modify|willSet|didSet [async] [throws…]`.
    fn accessor_spec(&self, node: Node<'_>) -> String {
        let mut mutation = String::new();
        let mut keyword = String::new();
        let mut is_async = false;
        let mut throws = String::new();

        let children: Vec<Node<'_>> = {
            let mut cursor = node.walk();
            node.children(&mut cursor).collect()
        };
        for child in children {
            let kind = child.kind();
            let text = self.text_of(child);
            if kind == node::MUTATION_MODIFIER {
                mutation = self.node_text(child);
            } else if kind == node::THROWS {
                throws = self.node_text(child);
            } else if kind == node::THROWS_CLAUSE {
                let parts = self.tokens(child);
                throws = format!("throws{}", join(&parts));
            } else if text == "async" {
                is_async = true;
            } else if matches!(text, "get" | "set" | "_modify" | "willSet" | "didSet")
                && keyword.is_empty()
            {
                keyword = text.to_owned();
            }
        }
        if keyword.is_empty() {
            keyword = match node.kind() {
                node::SETTER_SPECIFIER | node::COMPUTED_SETTER => "set".to_owned(),
                node::MODIFY_SPECIFIER | node::COMPUTED_MODIFY => "_modify".to_owned(),
                node::WILLSET_CLAUSE => "willSet".to_owned(),
                node::DIDSET_CLAUSE => "didSet".to_owned(),
                _ => "get".to_owned(),
            };
        }

        let mut parts = Vec::new();
        if !mutation.is_empty() {
            parts.push(mutation);
        }
        parts.push(keyword);
        if is_async {
            parts.push("async".to_owned());
        }
        if !throws.is_empty() {
            parts.push(throws);
        }
        parts.join(" ")
    }

    /// Find the specifier/clause node nested inside a computed accessor node.
    fn accessor_child<'t>(&self, node: Node<'t>) -> Option<Node<'t>> {
        let wanted = match node.kind() {
            node::COMPUTED_GETTER => node::GETTER_SPECIFIER,
            node::COMPUTED_SETTER => node::SETTER_SPECIFIER,
            node::COMPUTED_MODIFY => node::MODIFY_SPECIFIER,
            _ => return Some(node),
        };
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|c| c.kind() == wanted)
    }

    /// Accessor specifiers for a computed property, observer block, or protocol
    /// property requirement, in canonical order and with retained effects.
    pub(crate) fn accessor_specs(&self, node: Node<'_>) -> String {
        let mut specs = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                node::COMPUTED_GETTER | node::COMPUTED_SETTER | node::COMPUTED_MODIFY => {
                    if let Some(specifier) = self.accessor_child(child) {
                        specs.push(self.accessor_spec(specifier));
                    }
                }
                node::GETTER_SPECIFIER
                | node::SETTER_SPECIFIER
                | node::MODIFY_SPECIFIER
                | node::WILLSET_CLAUSE
                | node::DIDSET_CLAUSE => {
                    specs.push(self.accessor_spec(child));
                }
                _ => {}
            }
        }
        if specs.is_empty() {
            // A shorthand computed body such as `{ value * 2 }` is a getter.
            specs.push("get".to_owned());
        }
        specs.join(" ")
    }

    /// A grouped attribute Doc. Its top-level argument list uses the shared
    /// bracket-list shape so a long attribute structurally breaks within the
    /// 80-column budget instead of producing an overlong line.
    pub(crate) fn attribute_doc(&self, attribute: Node<'_>) -> Doc {
        let children: Vec<Node<'_>> = {
            let mut cursor = attribute.walk();
            attribute.children(&mut cursor).collect()
        };
        let open = children
            .iter()
            .position(|child| self.text_of(*child) == "(");
        let Some(open) = open else {
            return Doc::Text(self.stripped_text(attribute));
        };
        let mut depth = 0i32;
        let mut close = None;
        for (index, child) in children.iter().enumerate().skip(open) {
            match self.text_of(*child) {
                "(" => depth += 1,
                ")" => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            return Doc::Text(self.stripped_text(attribute));
        };

        // Prefix: `@` and the attribute path before the argument list.
        let mut prefix = Vec::new();
        for child in &children[..open] {
            self.push_tokens(*child, &mut prefix, true);
        }
        let mut segments: Vec<&[Node<'_>]> = Vec::new();
        let inner = &children[open + 1..close];
        let mut segment_start = 0usize;
        let mut segment_depth = 0i32;
        for (index, child) in inner.iter().enumerate() {
            match self.text_of(*child) {
                "(" | "[" | "{" => segment_depth += 1,
                ")" | "]" | "}" => segment_depth -= 1,
                "," if segment_depth == 0 => {
                    segments.push(&inner[segment_start..index]);
                    segment_start = index + 1;
                }
                _ => {}
            }
        }
        segments.push(&inner[segment_start..]);

        let arguments: Vec<Doc> = segments
            .into_iter()
            .filter_map(|segment| {
                let mut elems = Vec::new();
                for child in segment {
                    self.push_struct_elems(*child, &mut elems, true);
                }
                // A trailing comma (or a comment-only segment) leaves no
                // elements; dropping it normalizes dangling commas and keeps
                // comment placement from changing the projection.
                if elems.is_empty() {
                    None
                } else {
                    Some(elements_doc(&group_token_runs(elems)))
                }
            })
            .collect();

        Doc::Concat(vec![
            Doc::Text(join(&prefix)),
            Doc::Group(Box::new(bracket_list(&arguments, "(", ")"))),
        ])
    }

    /// Semantic final path segment of an attribute name, e.g. `Test` for both
    /// `@Test` and `@Testing.Test`. Backticks are lexical quoting, not part of
    /// the identifier, so `@`Test`` and `@Testing.`Test`` are detected as
    /// `@Test` too. Only detection uses this value; canonical rendering keeps
    /// the written spelling verbatim.
    pub(crate) fn attribute_name(&self, attribute: Node<'_>) -> String {
        let mut names = Vec::new();
        let mut cursor = attribute.walk();
        for child in attribute.named_children(&mut cursor) {
            if child.kind() == node::USER_TYPE {
                let mut inner = child.walk();
                for part in child.named_children(&mut inner) {
                    if part.kind() == node::TYPE_IDENTIFIER {
                        names.push(self.text_of(part).to_owned());
                    }
                }
            }
        }
        names
            .pop()
            .map(|name| semantic_identifier(&name).to_owned())
            .unwrap_or_default()
    }

    /// Text of the `name` field of a declaration, if present.
    pub(crate) fn name_text(&self, node: Node<'_>) -> Option<String> {
        node.child_by_field_name(field::NAME)
            .map(|name| self.node_text(name))
    }
}

/// Strip lexical backtick quoting from an identifier used for detection only.
/// The canonical text keeps the written spelling; this is never fed back into
/// rendering.
fn semantic_identifier(text: &str) -> &str {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('`') && text.ends_with('`') {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

fn collect_lambdas(node: Node<'_>, out: &mut Vec<(usize, usize)>) {
    if node.kind() == node::LAMBDA_LITERAL {
        out.push((node.start_byte(), node.end_byte()));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_lambdas(child, out);
    }
}

/// Convert a canonical token stream into structural elements. Only `(` and
/// `[` runs are grouped here: `<`/`>` are ambiguous in Swift (comparisons vs
/// generics), so genuine generic lists are produced from grammar nodes by
/// `push_struct_elems`, never from token heuristics.
fn tokens_to_elems(tokens: &[String]) -> Vec<Elem> {
    let mut elems = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        if let Some((open, close)) = bracket_pair(&tokens[index])
            && let Some(end) = matching_close(tokens, index, open, close)
        {
            let items: Vec<Doc> = split_list_items(&tokens[index + 1..end])
                .into_iter()
                .filter(|segment| !segment.is_empty())
                .map(|segment| elements_doc(&tokens_to_elems(&segment)))
                .collect();
            elems.push(Elem::List { items, open, close });
            index = end + 1;
            continue;
        }
        elems.push(Elem::Token(tokens[index].clone()));
        index += 1;
    }
    elems
}

/// Group consecutive plain tokens into `(...)`/`[...]` lists, leaving already
/// structural elements (notably generic `<...>` lists) untouched.
fn group_token_runs(elems: Vec<Elem>) -> Vec<Elem> {
    let mut out = Vec::new();
    let mut run: Vec<String> = Vec::new();
    for elem in elems {
        match elem {
            Elem::Token(token) => run.push(token),
            other => {
                if !run.is_empty() {
                    out.extend(tokens_to_elems(&run));
                    run.clear();
                }
                out.push(other);
            }
        }
    }
    if !run.is_empty() {
        out.extend(tokens_to_elems(&run));
    }
    out
}

fn bracket_pair(token: &str) -> Option<(&'static str, &'static str)> {
    match token {
        "(" => Some(("(", ")")),
        "[" => Some(("[", "]")),
        _ => None,
    }
}

fn matching_close(tokens: &[String], start: usize, open: &str, close: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (offset, token) in tokens.iter().enumerate().skip(start) {
        if token == open {
            depth += 1;
        } else if token == close {
            depth -= 1;
            if depth == 0 {
                return Some(offset);
            }
        }
    }
    None
}

/// Split the tokens inside one bracket pair on top-level commas. Empty
/// segments (a trailing comma, or a comment that produced no tokens) are
/// dropped by the caller so they cannot become phantom items.
fn split_list_items(tokens: &[String]) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0i32;
    for token in tokens {
        if matches!(token.as_str(), "(" | "[" | "{") {
            depth += 1;
        } else if matches!(token.as_str(), ")" | "]" | "}") {
            depth = (depth - 1).max(0);
        }
        if token == "," && depth == 0 {
            segments.push(std::mem::take(&mut current));
        } else {
            current.push(token.clone());
        }
    }
    segments.push(current);
    segments
}

enum Elem {
    Token(String),
    Doc {
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
    let mut parts: Vec<Doc> = Vec::new();
    let mut previous: Option<&str> = None;
    for element in elements {
        match element {
            Elem::Token(token) => {
                if let Some(previous) = previous
                    && needs_space(previous, token)
                {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                parts.push(Doc::Text(token.clone()));
                previous = Some(token);
            }
            Elem::Doc { doc, first, last } => {
                if let Some(previous) = previous
                    && needs_space(previous, first)
                {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                parts.push(doc.clone());
                previous = Some(last);
            }
            Elem::List { items, open, close } => {
                // A generic `<...>` list attaches to the preceding type name,
                // and after it `?`, `)`, `]`, or `,` attach tightly. `>` is a
                // spaced operator only when it is a comparison token, so use a
                // virtual closing delimiter for the post-generic spacing.
                if *open != "<"
                    && let Some(previous) = previous
                    && needs_space(previous, open)
                {
                    parts.push(Doc::Text(" ".to_owned()));
                }
                // Each list decides independently whether it fits, so a long
                // parameter list breaks without dragging a short generic list
                // that precedes it onto multiple lines.
                parts.push(Doc::Group(Box::new(bracket_list(items, open, close))));
                previous = if *open == "<" { Some(")") } else { Some(close) };
            }
        }
    }
    Doc::Concat(parts)
}

/// The shared `base::doc` bracket-list shape. Renders flat when it fits the
/// 80-column budget and broken one item per line (with a trailing comma)
/// otherwise. `Broken(",")` and `SoftNil` only emit when broken.
pub(crate) fn bracket_list(items: &[Doc], open: &str, close: &str) -> Doc {
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

pub(crate) fn container_doc(header: Doc, members: Vec<Doc>) -> Doc {
    if members.is_empty() {
        return Doc::Group(Box::new(Doc::Concat(vec![
            header,
            Doc::Text(" {}".to_owned()),
        ])));
    }
    let mut body = Vec::new();
    for (index, member) in members.into_iter().enumerate() {
        if index > 0 {
            body.push(Doc::Line);
        }
        body.push(member);
    }
    Doc::Concat(vec![
        Doc::Group(Box::new(Doc::Concat(vec![
            header,
            Doc::Text(" {".to_owned()),
        ]))),
        Doc::Line,
        Doc::Indent(Box::new(Doc::Concat(body))),
        Doc::Line,
        Doc::Text("}".to_owned()),
    ])
}

/// Compose attributes with a declaration. Exactly one attribute stays inline
/// when it fits (`Doc::Group` + `SoftLine`); two or more render one per line.
pub(crate) fn with_attributes(attributes: Vec<Doc>, declaration: Doc) -> Doc {
    match attributes.len() {
        0 => declaration,
        1 => Doc::Group(Box::new(Doc::Concat(vec![
            attributes.into_iter().next().expect("one attribute"),
            Doc::SoftLine,
            declaration,
        ]))),
        _ => {
            let mut parts = Vec::new();
            for attribute in attributes {
                parts.push(attribute);
                parts.push(Doc::Line);
            }
            parts.push(declaration);
            Doc::Concat(parts)
        }
    }
}

#[cfg(test)]
mod tests {
    use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
    use tree_sitter::Node;

    use super::{Renderer, join, lex, needs_space, render};
    use crate::SwiftProjector;

    fn find_node<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
        if node.kind() == kind {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_node(child, kind) {
                return Some(found);
            }
        }
        None
    }

    fn project(source: &str, mode: ProjectionMode) -> String {
        let path = SupportedPath::new(RepoPath::new("src/Test.swift").unwrap()).unwrap();
        let file = SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode,
            })
            .unwrap_or_else(|error| panic!("projection failed: {error}"));
        file.canonical_text().to_owned()
    }

    fn signatures(source: &str) -> String {
        project(source, ProjectionMode::Signatures)
    }

    fn types(source: &str) -> String {
        project(source, ProjectionMode::Types)
    }

    fn tests(source: &str) -> String {
        project(source, ProjectionMode::Tests)
    }

    fn spaced(parts: &[&str]) -> String {
        join(
            &parts
                .iter()
                .map(|part| (*part).to_owned())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn signature_tokens_are_canonical() {
        assert_eq!(spaced(&["var", "x", ":", "Double"]), "var x: Double");
        assert_eq!(
            spaced(&["static", "let", "origin", ":", "Point"]),
            "static let origin: Point"
        );
        assert_eq!(
            spaced(&["struct", "Point", ":", "Equatable"]),
            "struct Point: Equatable"
        );
        assert_eq!(spaced(&["Item", ".", "ID", "==", "Int"]), "Item.ID == Int");
        // Flat generic tokens attach; comparison operators are spaced by the
        // grammar-aware leaf walker (verified in the projector tests below).
        assert_eq!(
            spaced(&["Result", "<", "Value", ",", "Error", ">"]),
            "Result<Value, Error>"
        );
        assert_eq!(spaced(&["[", "Item", "]"]), "[Item]");
        assert_eq!(spaced(&["->", "Item", "?"]), "-> Item?");
        assert_eq!(spaced(&["Codable", "&", "Sendable"]), "Codable & Sendable");
        assert_eq!(spaced(&["(", "T", ")", "->", "Void"]), "(T) -> Void");
        assert_eq!(
            spaced(&["@escaping", "(", "Result", ")"]),
            "@escaping (Result)"
        );
    }

    #[test]
    fn condition_guard_normalization_collapses_spelling_variants() {
        let a = join(&lex("os(iOS)"));
        let b = join(&lex("os ( iOS )"));
        assert_eq!(a, "os(iOS)");
        assert_eq!(b, a);
        assert_eq!(join(&lex("canImport ( UIKit )")), "canImport(UIKit)");
        assert_eq!(join(&lex("DEBUG || os(macOS)")), "DEBUG || os(macOS)");
    }

    #[test]
    fn literal_text_spacing_is_preserved() {
        assert_eq!(
            spaced(&[
                "\"", "values: ", "\\(", "items", ".", "map", "{ ... }", ".", "joined", "(", ")",
                ")", " end", "\""
            ]),
            "\"values: \\(items.map { ... }.joined()) end\""
        );
    }

    #[test]
    fn needs_space_attaches_postfix_punctuation() {
        assert!(!needs_space("x", ":"));
        assert!(needs_space(":", "Int"));
        assert!(!needs_space("foo", "("));
        assert!(needs_space("if", "("));
        assert!(!needs_space(")", ","));
        assert!(needs_space(",", "b"));
    }

    #[test]
    fn lex_merges_operators_invariantly_under_whitespace() {
        assert_eq!(join(&lex("a==b")), join(&lex("a == b")));
        assert_eq!(join(&lex("a==b")), "a == b");
        assert_eq!(join(&lex("a->b")), join(&lex("a -> b")));
        assert_eq!(join(&lex("x>=1")), "x >= 1");
        assert_eq!(
            join(&lex("DEBUG||os(macOS)")),
            join(&lex("DEBUG || os ( macOS )"))
        );
    }

    #[test]
    fn generic_comments_do_not_create_phantom_commas() {
        let plain = signatures("func f<T, U>(_ x: T, _ y: U) {}\n");
        let commented = signatures("func f<T /* note */, U>(_ x: T, _ y: U) {}\n");
        assert_eq!(commented, plain);
        assert_eq!(plain, "func f<T, U>(_ x: T, _ y: U) { ... }\n");
        assert!(!commented.contains(", ,"), "{commented}");
    }

    #[test]
    fn parameter_defaults_survive_comments_between_siblings() {
        let plain = signatures("func f(x: Int = 3) {}\n");
        let commented = signatures("func f(x: Int /*note*/ = /*default*/ 3) {}\n");
        assert_eq!(commented, plain);
        assert_eq!(plain, "func f(x: Int = 3) { ... }\n");
    }

    #[test]
    fn dangling_trailing_commas_are_normalized_in_every_retained_list() {
        assert_eq!(
            signatures("func f(a: Int, b: Int,) {}\n"),
            "func f(a: Int, b: Int) { ... }\n"
        );
        assert_eq!(types("struct S<T, U,> {}\n"), "struct S<T, U> {}\n");
        assert_eq!(
            types("@foo(a: 1, b: 2,) struct S {}\n"),
            "@foo(a: 1, b: 2) struct S {}\n"
        );
    }

    #[test]
    fn interpolation_executable_expression_is_canonicalized() {
        let a = "@foo(v: \"\\(items.map { print($0) }.joined())\")\nstruct S {}\n";
        let b = "@foo(v: \"\\(items.map { other($0, 2) } /* c */  .joined() )\")\nstruct S {}\n";
        assert_eq!(types(a), types(b));
        let out = types(a);
        assert_eq!(
            out,
            "@foo(v: \"\\(items.map { ... }.joined())\") struct S {}\n"
        );
        assert!(!out.contains("print("), "{out}");
        assert!(!types(b).contains("/* c */"), "{out}");
    }

    #[test]
    fn literal_text_outside_interpolation_is_verbatim() {
        let out = types("@foo(v: \"http://site // not a comment\")\nstruct S {}\n");
        assert!(out.contains("\"http://site // not a comment\""), "{out}");
        let raw = types("@foo(v: #\"raw // \\(literal) tail\"#)\nstruct S {}\n");
        assert!(raw.contains("#\"raw // \\(literal) tail\"#"), "{raw}");
        let regex = types("@foo(v: /a+b/)\nstruct S {}\n");
        assert!(regex.contains("/a+b/"), "{regex}");
    }

    #[test]
    fn nested_lists_break_structurally_over_the_budget() {
        let a = "x".repeat(40);
        let b = "y".repeat(40);
        let c = "z".repeat(40);
        let source = format!("func f(xs: [String] = [\"{a}\", \"{b}\", \"{c}\"]) {{}}\n");
        let out = signatures(&source);
        assert!(out.contains("[\n"), "nested list should break: {out}");
        assert!(out.contains(&format!("        \"{a}\",")), "{out}");
        assert!(out.contains("    ],"), "{out}");
    }

    #[test]
    fn backtick_attribute_names_are_detected_without_changing_canonical_text() {
        let plain = tests("@Test func basic() {}\n");
        assert_eq!(plain, "@Test func basic() { ... }\n");
        for source in [
            "@`Test` func basic() {}\n",
            "@Testing.`Test` func basic() {}\n",
        ] {
            let out = tests(source);
            assert!(!out.is_empty(), "{source} must be detected as a test");
            assert!(out.contains("func basic() { ... }"), "{out}");
        }

        // Detection is semantic; the written spelling stays in the projection.
        assert!(
            signatures("@`Test` func basic() {}\n").contains("@`Test`"),
            "canonical text must keep the written backticks"
        );
        assert!(
            signatures("@Testing.`Test` func basic() {}\n").contains("@Testing.`Test`"),
            "canonical text must keep the written qualified backticks"
        );
    }

    #[test]
    fn node_doc_is_available_for_split_pieces() {
        let source = "func f(_ x: Result<Value, Error>) {}\n";
        let path = SupportedPath::new(RepoPath::new("src/Test.swift").unwrap()).unwrap();
        let tree = crate::syntax::parse(source, &path).expect("parse");
        let node = find_node(tree.root_node(), "user_type").expect("a user type");
        let renderer = Renderer::new(&path, source);
        assert_eq!(render(&renderer.node_doc(node), 0), "Result<Value, Error>");
    }

    #[test]
    fn comparison_operators_are_never_generic_lists() {
        let spaced = signatures("func f(_ b: Bool = a < b && c > d) {}\n");
        let tight = signatures("func f(_ b: Bool = a<b&&c>d) {}\n");
        assert_eq!(spaced, tight);
        assert_eq!(spaced, "func f(_ b: Bool = a < b && c > d) { ... }\n");

        let attr_spaced = types("@foo(v: 1 < 2 && 3 > 4) struct S {}\n");
        let attr_tight = types("@foo(v:1<2&&3>4) struct S {}\n");
        assert_eq!(attr_spaced, attr_tight);
        assert_eq!(attr_spaced, "@foo(v: 1 < 2 && 3 > 4) struct S {}\n");
    }

    #[test]
    fn ternary_operators_are_spaced_without_changing_optional_types_or_annotations() {
        let canonical = "func f(_ x: Int = flag ? 1 : 2) { ... }\n";
        assert_eq!(
            signatures("func f(_ x: Int = flag ? 1 : 2) {}\n"),
            canonical
        );
        assert_eq!(
            signatures("func f(_ x: Int = flag  ?  1  :  2) {}\n"),
            canonical
        );
        assert_eq!(
            types("@foo(v: flag ? 1 : 2) struct S {}\n"),
            "@foo(v: flag ? 1 : 2) struct S {}\n"
        );

        assert_eq!(
            signatures("func h(_ x: Int? = nil) {}\n"),
            "func h(_ x: Int? = nil) { ... }\n"
        );
        assert_eq!(
            types("struct S { var x: Int? }\n"),
            "struct S {\n    var x: Int?\n}\n"
        );
        assert_eq!(
            signatures("func d(_ x: [String: Int] = [:]) {}\n"),
            "func d(_ x: [String: Int] = [:]) { ... }\n"
        );
    }

    #[test]
    fn nested_ternary_branches_strip_closures_at_every_depth() {
        let a = signatures("func f(_ x: Int = flag ? { secretA() } : { secretB() }) {}\n");
        let b = signatures("func f(_ x: Int = flag ? { otherA() } : { otherB() }) {}\n");
        assert_eq!(a, b);
        assert!(!a.contains("secretA"), "{a}");
        assert!(!a.contains("secretB"), "{a}");
        assert!(a.contains("flag ? { ... } : { ... }"), "{a}");

        let nested = signatures("func g(_ x: Int = a ? b ? 1 : 2 : 3) {}\n");
        assert_eq!(nested, "func g(_ x: Int = a ? b ? 1 : 2 : 3) { ... }\n");
    }

    #[test]
    fn long_comparison_default_breaks_without_inserting_commas() {
        let source = "func f(_ flag: Bool = someVeryLongIdentifierName < anotherVeryLongIdentifierName && thirdVeryLongIdentifierName > fourthVeryLongIdentifierName) {}\n";
        let out = signatures(source);
        let expression = "someVeryLongIdentifierName < anotherVeryLongIdentifierName && thirdVeryLongIdentifierName > fourthVeryLongIdentifierName";
        assert!(out.contains(expression), "{out}");
        assert_eq!(out.matches(',').count(), 1, "{out}");
        assert!(out.starts_with("func f(\n    _ flag: Bool = "), "{out}");
    }

    #[test]
    fn nested_generic_lists_still_render_and_wrap() {
        let short = signatures(
            "func make() -> Result<Value, Dictionary<String, Array<Int>>> { fatalError() }\n",
        );
        assert!(
            short.contains("Result<Value, Dictionary<String, Array<Int>>> { ... }"),
            "{short}"
        );

        let long = signatures(
            "func make() -> Dictionary<String, Array<Result<VeryLongTypeName, AnotherLongErrorType>>> { fatalError() }\n",
        );
        assert!(long.contains("Dictionary<"), "{long}");
        assert!(long.contains("AnotherLongErrorType"), "{long}");
        assert!(
            !long.contains("VeryLongTypeName AnotherLongErrorType"),
            "{long}"
        );
    }

    #[test]
    fn raw_string_interpolation_is_canonicalized_without_touching_literal_text() {
        let a = types("@foo(v: #\"\\#(items.map { secret($0) }.joined())\"#)\nstruct S {}\n");
        let b = types("@foo(v: #\"\\#(items.map { other($0, 2) }.joined())\"#)\nstruct S {}\n");
        assert_eq!(a, b);
        assert!(!a.contains("secret"), "{a}");
        assert!(!a.contains("other("), "{a}");
        assert!(a.contains("#\"\\#(items.map { ... }.joined())\"#"), "{a}");

        let c = types("@foo(v: ##\"\\##(a   +   b)\"##)\nstruct S {}\n");
        let d = types("@foo(v: ##\"\\##(a+b)\"##)\nstruct S {}\n");
        assert_eq!(c, d);
        assert!(c.contains("##\"\\##(a + b)\"##"), "{c}");

        let e = types("@foo(v: #\"literal \\( not interpolation\"#)\nstruct S {}\n");
        assert!(e.contains("#\"literal \\( not interpolation\"#"), "{e}");
    }

    #[test]
    fn node_doc_strips_closures_in_enum_payload_defaults() {
        let source = "enum E { case cb(action: () -> Void = { secret() }) }\n";
        let path = SupportedPath::new(RepoPath::new("src/Test.swift").unwrap()).unwrap();
        let tree = crate::syntax::parse(source, &path).expect("parse");
        let payload = find_node(tree.root_node(), "enum_type_parameters").expect("payload");
        let renderer = Renderer::new(&path, source);
        let out = render(&renderer.node_doc(payload), 0);
        assert!(!out.contains("secret"), "closure body leaked: {out}");
        assert_eq!(out, "(action: () -> Void = { ... })");
    }
}
