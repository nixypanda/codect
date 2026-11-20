//! Swift language adapter for Codect.
//!
//! Owns the Swift parser constructor and all Swift grammar node-kind knowledge,
//! and implements the Codect language projector interface for Swift source
//! files. Parsing, extraction, and canonical rendering are all-or-nothing: any
//! error or missing node anywhere (including inside a stripped body), an
//! unsupported `_read` accessor, or a malformed conditional sequence is fatal.
//!
//! This module also retains the two Gate A reference algorithms that the frozen
//! contract specifies and that the grammar-probe tests exercise directly:
//!   * `plan_conditionals` — token-aware reconstruction and validation of flat
//!     `#if`/`#elseif`/`#else`/`#endif` siblings in every declaration container.
//!   * `read_in_accessor_position` — narrow detection of an unsupported `_read`
//!     accessor that the grammar misparses as a clean call expression.

pub mod extract;
pub mod render;
pub mod syntax;

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use base::{
    Language, LanguageProjector, ProjectedFile, ProjectionError, ProjectionInput, RepoPath,
};
use tree_sitter::{Node, Parser, Tree};

/// The Swift projector.
#[derive(Clone, Copy, Debug, Default)]
pub struct SwiftProjector;

impl SwiftProjector {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageProjector for SwiftProjector {
    fn language(&self) -> Language {
        Language::Swift
    }

    fn supports_path(&self, path: &RepoPath) -> bool {
        path.is_swift()
    }

    fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
        extract::project_file(input)
    }
}

/// A parse report for one source string.
#[derive(Debug)]
pub struct ParseReport {
    /// True when the grammar produced a tree with no ERROR/MISSING nodes.
    pub ok: bool,
    /// S-expressions of the first few error/missing nodes, for diagnostics.
    pub error_nodes: Vec<String>,
    /// The full tree S-expression.
    pub sexp: String,
    /// Unique named node kinds observed.
    pub named_kinds: BTreeSet<String>,
    /// Unique field names observed.
    pub fields: BTreeSet<String>,
}

impl ParseReport {
    pub fn for_source(source: &str) -> Result<Self, AssignError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .map_err(|_| AssignError)?;
        let tree = parser.parse(source, None).ok_or(AssignError)?;
        Ok(Self::from_tree(&tree))
    }

    pub fn from_tree(tree: &Tree) -> Self {
        let root = tree.root_node();
        let mut error_nodes = Vec::new();
        let mut named_kinds = BTreeSet::new();
        let mut fields = BTreeSet::new();
        collect(root, &mut error_nodes, &mut named_kinds, &mut fields);
        Self {
            ok: error_nodes.is_empty(),
            error_nodes,
            sexp: root.to_sexp(),
            named_kinds,
            fields,
        }
    }
}

/// The grammar could not be assigned to a Tree-sitter parser. In a real adapter
/// this must be a fatal `ProjectionError`; in the spike it is a distinct error so
/// ABI failures are never confused with syntax failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssignError;

impl fmt::Display for AssignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tree-sitter-swift grammar could not be assigned to a parser")
    }
}

impl std::error::Error for AssignError {}

fn collect(
    node: Node<'_>,
    errors: &mut Vec<String>,
    kinds: &mut BTreeSet<String>,
    fields: &mut BTreeSet<String>,
) {
    if (node.is_error() || node.is_missing()) && errors.len() < 8 {
        errors.push(format!(
            "{} [{}..{}]",
            node.kind(),
            node.start_byte(),
            node.end_byte()
        ));
    }
    if node.is_named() {
        kinds.insert(node.kind().to_owned());
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(field) = cursor.field_name() {
                fields.insert(field.to_owned());
            }
            collect(cursor.node(), errors, kinds, fields);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn descendants<'tree>(root: Node<'tree>) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        let children: Vec<Node<'tree>> = node.children(&mut cursor).collect();
        for child in children {
            stack.push(child);
        }
        out.push(node);
    }
    out
}

// ---------------------------------------------------------------------------
// Reference algorithm 1: conditional-compilation reconstruction.
//
// The grammar emits flat `directive` siblings in source_file, class_body,
// enum_class_body, and protocol_body. It does not group branches and it accepts
// malformed directive sequences without error nodes. The adapter must therefore
// reconstruct a conditional tree from the flat tokens and validate it itself.
// ---------------------------------------------------------------------------

/// The four conditional directives, recognized from a `directive` node's source
/// text (token-aware: the grammar gives only a bare `directive` kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectiveKind {
    If,
    ElseIf,
    Else,
    EndIf,
}

impl DirectiveKind {
    pub fn from_text(text: &str) -> Option<Self> {
        let word = text
            .trim_start()
            .trim_start_matches('#')
            .split_whitespace()
            .next()?;
        match word {
            "if" => Some(Self::If),
            "elseif" => Some(Self::ElseIf),
            "else" => Some(Self::Else),
            "endif" => Some(Self::EndIf),
            _ => None,
        }
    }
}

/// A flat directive token with its normalized header and guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveInfo {
    pub kind: DirectiveKind,
    /// Normalized original header, e.g. `#if canImport(UIKit)`, `#else`.
    pub header: String,
    /// Guard token used in keys: `canImport(UIKit)`, `os(macOS)`, `else`.
    pub guard: String,
    /// Zero-based byte span of the directive node.
    pub span: (usize, usize),
    /// Kind of the declaration container that directly owns this directive.
    pub parent_container: String,
}

/// A structural validation error the grammar does not report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CondError {
    Unmatched(DirectiveKind, (usize, usize)),
    DuplicateElse((usize, usize)),
    ElseIfAfterElse((usize, usize)),
    MissingEndIf((usize, usize)),
}

/// Role of a reconstructed conditional item. `Group` and `Branch` are synthetic
/// `ItemKind::Module` containers in the real model; `Member` is a projected
/// declaration keyed by extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondRole {
    Group,
    Branch,
    Member,
}

/// One flat item of the reconstructed conditional tree. `parent_key` chains are
/// explicit and directly usable as `ProjectedItem` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CondItem {
    pub key: String,
    pub parent_key: Option<String>,
    pub role: CondRole,
    /// Present for `Group`/`Branch`; the original Swift-style header.
    pub header: Option<String>,
    pub span: (usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalPlan {
    pub directives: Vec<DirectiveInfo>,
    pub items: Vec<CondItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    Assign(AssignError),
    Validation(Vec<CondError>),
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assign(e) => write!(f, "{e}"),
            Self::Validation(errors) => write!(f, "invalid conditional directives: {errors:?}"),
        }
    }
}

impl std::error::Error for PlanError {}

#[derive(Debug)]
struct Frame {
    group_key: String,
    start: usize,
    branches: Vec<BranchFrame>,
}

#[derive(Debug)]
struct BranchFrame {
    key: String,
    header: String,
    start: usize,
    end: usize,
    seen_member: bool,
}

/// Reconstruct conditional groups/branches for one source string.
pub fn plan_conditionals(source: &str) -> Result<ConditionalPlan, PlanError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .map_err(|_| PlanError::Assign(AssignError))?;
    let tree = parser
        .parse(source, None)
        .ok_or(PlanError::Assign(AssignError))?;
    if tree.root_node().has_error() {
        return Err(PlanError::Validation(vec![CondError::Unmatched(
            DirectiveKind::If,
            (tree.root_node().start_byte(), tree.root_node().end_byte()),
        )]));
    }

    let mut directives = Vec::new();
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut ordinals: BTreeMap<String, usize> = BTreeMap::new();
    let all = descendants(tree.root_node());
    let containers: Vec<Node<'_>> = all
        .into_iter()
        .filter(|n| {
            matches!(
                n.kind(),
                "source_file" | "class_body" | "enum_class_body" | "protocol_body"
            )
        })
        .collect();

    for (container_index, container) in containers.iter().enumerate() {
        let container_key = if *container == tree.root_node() {
            "FILE".to_owned()
        } else {
            format!("FILE::{container_index}::{}", container.kind())
        };
        let mut cursor = container.walk();
        let children: Vec<Node<'_>> = container.children(&mut cursor).collect();
        let mut stack: Vec<Frame> = Vec::new();
        let mut top_members: usize = 0;

        for child in children {
            if child.kind() == "diagnostic" {
                // `#error`/`#warning` are dropped and never make a branch retained.
                continue;
            }
            if child.kind() != "directive" {
                let parent = stack
                    .last_mut()
                    .and_then(|f| f.branches.last_mut())
                    .map(|b| {
                        b.end = child.end_byte();
                        b.seen_member = true;
                        b.key.clone()
                    });
                let end = child.end_byte();
                let owner = parent.clone().unwrap_or_else(|| container_key.clone());
                let base = if parent.is_some() {
                    format!("{owner}::member::{}:{}", top_members, child.kind())
                } else {
                    top_members += 1;
                    format!("{owner}::member::{top_members}:{}", child.kind())
                };
                items.push(CondItem {
                    key: unique(&mut ordinals, base),
                    parent_key: parent,
                    role: CondRole::Member,
                    header: None,
                    span: (child.start_byte(), end),
                });
                continue;
            }

            let text = &source[child.start_byte()..child.end_byte()];
            let Some(kind) = DirectiveKind::from_text(text) else {
                continue;
            };
            let header = collapse_ws(text);
            let bare = header.trim_start_matches('#');
            let guard = match bare.split_once(' ') {
                Some((keyword, rest)) => format!("{keyword}:{}", collapse_ws(rest)),
                None => bare.to_owned(), // `#else`, `#endif`
            };
            let span = (child.start_byte(), child.end_byte());
            directives.push(DirectiveInfo {
                kind,
                header: header.clone(),
                guard: guard.clone(),
                span,
                parent_container: container.kind().to_owned(),
            });

            match kind {
                DirectiveKind::If => {
                    let base = format!("{container_key}::cond::{guard}");
                    let key = unique(&mut ordinals, base);
                    let parent_key = stack
                        .last()
                        .and_then(|f| f.branches.last())
                        .map(|b| b.key.clone());
                    items.push(CondItem {
                        key: key.clone(),
                        parent_key: parent_key.clone(),
                        role: CondRole::Group,
                        header: Some(header.clone()),
                        span,
                    });
                    let branch_key = format!("{key}::branch::0::{guard}");
                    stack.push(Frame {
                        group_key: key,
                        start: child.start_byte(),
                        branches: vec![BranchFrame {
                            key: branch_key,
                            header,
                            start: child.start_byte(),
                            end: child.end_byte(),
                            seen_member: false,
                        }],
                    });
                }
                DirectiveKind::ElseIf | DirectiveKind::Else => {
                    let Some(frame) = stack.last_mut() else {
                        errors.push(CondError::Unmatched(kind, span));
                        continue;
                    };
                    if kind == DirectiveKind::ElseIf
                        && frame.branches.last().is_some_and(|b| b.header == "#else")
                    {
                        errors.push(CondError::ElseIfAfterElse(span));
                        continue;
                    }
                    if kind == DirectiveKind::Else
                        && frame.branches.iter().any(|b| b.header == "#else")
                    {
                        errors.push(CondError::DuplicateElse(span));
                        continue;
                    }
                    let index = frame.branches.len();
                    frame.branches.push(BranchFrame {
                        key: format!("{}::branch::{index}::{}", frame.group_key, guard),
                        header,
                        start: child.start_byte(),
                        end: child.end_byte(),
                        seen_member: false,
                    });
                }
                DirectiveKind::EndIf => {
                    let Some(frame) = stack.pop() else {
                        errors.push(CondError::Unmatched(kind, span));
                        continue;
                    };
                    // Branch items are emitted only at `#endif`, once the full
                    // header list (including empty branches) is known.
                    for branch in &frame.branches {
                        items.push(CondItem {
                            key: branch.key.clone(),
                            parent_key: Some(frame.group_key.clone()),
                            role: CondRole::Branch,
                            header: Some(branch.header.clone()),
                            span: (branch.start, branch.end),
                        });
                    }
                    if let Some(group) = items.iter_mut().find(|i| i.key == frame.group_key) {
                        group.span.1 = child.end_byte();
                    }
                }
            }
        }

        for frame in &stack {
            errors.push(CondError::MissingEndIf((frame.start, source.len())));
        }
    }

    if errors.is_empty() {
        Ok(ConditionalPlan { directives, items })
    } else {
        Err(PlanError::Validation(errors))
    }
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in text.trim().chars() {
        if ch.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            out.push(ch);
            pending_space = false;
        }
    }
    out
}

fn unique(ordinals: &mut BTreeMap<String, usize>, base: String) -> String {
    let count = ordinals.entry(base.clone()).or_insert(0);
    let key = if *count == 0 {
        base
    } else {
        format!("{base}~{count}")
    };
    *count += 1;
    key
}

// ---------------------------------------------------------------------------
// Reference algorithm 2: narrow `_read` accessor detection.
//
// The grammar has no `computed_read` node. `_read { ... }` alone is accepted as a
// `computed_property` whose direct `statements` child is a call expression on
// `_read`; a `_read(...)` call inside a *real* getter body is nested under
// `computed_getter` and must not be flagged. Detection therefore looks only at
// direct children of a `computed_property`/`protocol_property_requirements`.
// ---------------------------------------------------------------------------

/// Detect the `_read` misparse from an already-parsed tree (no second parse).
///
/// The grammar has no `computed_read` node. `_read { ... }` alone is accepted as
/// a `computed_property` whose direct `statements` child is a `call_expression`
/// whose callee is exactly `_read` with an accessor-shaped trailing lambda.
/// An ordinary zero-argument call `_read()` and `_read(1)` are legal calls and
/// must not be flagged, nor may a `_read { ... }` nested in a real getter body.
pub(crate) fn read_in_accessor_position_node(root: Node<'_>, source: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for node in descendants(root) {
        if !matches!(
            node.kind(),
            "computed_property" | "protocol_property_requirements"
        ) {
            continue;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "statements" {
                continue;
            }
            let mut statements = child.walk();
            for expression in child.named_children(&mut statements) {
                if expression.kind() == "call_expression"
                    && is_read_accessor_call(expression, source)
                {
                    found.push((expression.start_byte(), expression.end_byte()));
                    break;
                }
            }
        }
    }
    found
}

/// True only for the exact identifier `_read` followed by an accessor-shaped
/// trailing lambda (`_read { yield storage }`). `_read()` and `_read(1)` are
/// ordinary calls, not accessors.
fn is_read_accessor_call(call: Node<'_>, source: &str) -> bool {
    let Some(callee) = call.named_child(0) else {
        return false;
    };
    if callee.kind() != "simple_identifier" || source.get(callee.byte_range()) != Some("_read") {
        return false;
    }
    let mut cursor = call.walk();
    for child in call.children(&mut cursor) {
        if child.kind() != "call_suffix" {
            continue;
        }
        let mut suffix = child.walk();
        for part in child.named_children(&mut suffix) {
            if part.kind() == "lambda_literal" {
                return true;
            }
        }
    }
    false
}

/// Byte spans of unsupported `_read` accessors in accessor position.
///
/// This public probe helper parses its own source; production projection calls
/// [`read_in_accessor_position_node`] on the tree it already parsed.
pub fn read_in_accessor_position(source: &str) -> Result<Vec<(usize, usize)>, AssignError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
        .map_err(|_| AssignError)?;
    let tree = parser.parse(source, None).ok_or(AssignError)?;
    Ok(read_in_accessor_position_node(tree.root_node(), source))
}

#[cfg(test)]
mod tests {
    use base::{LanguageProjector, RepoPath};

    use super::{ParseReport, SwiftProjector};

    #[test]
    fn grammar_assigns_to_tree_sitter_027() {
        let report = ParseReport::for_source("let x = 1\n").expect("grammar assignment failed");
        assert!(report.ok, "basic Swift failed to parse: {}", report.sexp);
    }

    #[test]
    fn read_detector_matches_only_the_exact_accessor_shaped_callee() {
        let source = "func _read(_ x: Int) -> Int { x }\nfunc _reader(_ x: Int) -> Int { x }\nstruct S {\n    var a: Int { _read { yield a } }\n    var b: Int { _read() }\n    var c: Int { _read(1) }\n    var d: Int { _reader() }\n    var e: Int { get { _read(1) } }\n}\n";
        let spans = super::read_in_accessor_position(source).expect("parse");
        assert_eq!(
            spans.len(),
            1,
            "only `_read {{ ... }}` is an accessor; `_read()` and `_read(1)` are calls"
        );
        for (start, end) in &spans {
            let text = &source[*start..*end];
            assert!(
                text.starts_with("_read") && text.contains('{'),
                "unexpected flagged text {text:?}"
            );
        }
        // The public helper and the node-based helper must agree.
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .expect("grammar assigns");
        let tree = parser.parse(source, None).expect("parse");
        assert_eq!(
            super::read_in_accessor_position_node(tree.root_node(), source),
            spans
        );
    }

    #[test]
    fn swift_projector_selects_case_sensitive_swift_paths() {
        let projector = SwiftProjector::new();
        assert_eq!(projector.language(), base::Language::Swift);
        assert!(projector.supports_path(&RepoPath::new("src/App.swift").unwrap()));
        assert!(!projector.supports_path(&RepoPath::new("src/App.SWIFT").unwrap()));
        assert!(!projector.supports_path(&RepoPath::new("src/lib.rs").unwrap()));
    }
}
