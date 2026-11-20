//! Inspection of a parsed Swift syntax tree into the shared projection model.
//!
//! Keys are allocated once, in a Signatures-superset pass over the whole file,
//! before any mode filtering, so a key never depends on whether a sibling was
//! dropped in the requested mode. Declaration containers, conditional groups
//! and branches are preserved as `ProjectedItem`s with valid parent chains;
//! executable bodies are never traversed.

use std::collections::BTreeSet;

use base::{
    ItemKind, KeyAllocator, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput,
    ProjectionMode, SourceSpan,
};
use tree_sitter::Node;

use crate::DirectiveKind;
use crate::render::{self, Doc, Renderer};
use crate::syntax::{self, field, node, span_for_bytes};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    File,
    Member,
    Protocol,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModeSet {
    types: bool,
    signatures: bool,
    tests: bool,
}

impl ModeSet {
    const NONE: Self = Self {
        types: false,
        signatures: false,
        tests: false,
    };
    /// Types and Signatures ("type-level shape").
    const TS: Self = Self {
        types: true,
        signatures: true,
        tests: false,
    };
    /// Signatures only.
    const SIG: Self = Self {
        types: false,
        signatures: true,
        tests: false,
    };
    /// Signatures and Tests (a detected test callable).
    fn contains(self, mode: ProjectionMode) -> bool {
        match mode {
            ProjectionMode::Types => self.types,
            ProjectionMode::Signatures => self.signatures,
            ProjectionMode::Tests => self.tests,
        }
    }

    fn with_tests(mut self) -> Self {
        self.tests = true;
        self
    }
}

enum RenderKind {
    /// A simple declaration; no nested declaration items.
    Leaf(Doc),
    /// A type/extension container. The header is stored; members are supplied
    /// at materialization time so filtering happens before text composition.
    Container(Doc),
    /// A conditional group. Its children are branches.
    Group,
    /// A conditional branch. Its children are retained members; its header is
    /// always emitted when the group survives.
    Branch { header: String },
}

struct Item {
    key: String,
    parent: Option<String>,
    kind: ItemKind,
    name: String,
    span: SourceSpan,
    depth: usize,
    render: RenderKind,
    children: Vec<Item>,
    modes: ModeSet,
}

struct Materialized {
    doc: Doc,
    items: Vec<ProjectedItem>,
}

struct Context<'a> {
    renderer: Renderer<'a>,
    source: &'a str,
    path: base::SupportedPath,
    keys: KeyAllocator,
    xctest_classes: BTreeSet<String>,
}

impl Context<'_> {
    fn unique(&mut self, base: String) -> String {
        self.keys.unique(base)
    }
}

pub(crate) fn project_file(input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
    let tree = syntax::parse(input.source, input.path)?;
    let root = tree.root_node();
    let mut context = Context {
        renderer: Renderer::new(input.path, input.source),
        source: input.source,
        path: input.path.clone(),
        keys: KeyAllocator::new(),
        xctest_classes: collect_xctest_classes(root, input.source),
    };
    let file_key = input.path.to_string();

    let items = build_container(root, None, &file_key, 0, Scope::File, false, &mut context)?;

    let mut projected = Vec::new();
    for item in &items {
        if let Some(materialized) = materialize(item, input.mode) {
            projected.extend(materialized.items);
        }
    }

    ProjectedFile::try_new(input.path.clone(), projected)
}

// ---------------------------------------------------------------------------
// XCTest context
// ---------------------------------------------------------------------------

/// Lexically qualified names of classes in this file that directly name
/// `XCTestCase` as a base. Only declaration containers are walked: a class
/// declared inside an executable body is a local type and must not influence
/// the Tests projection, and nested names must not contaminate extensions of
/// unrelated classes with the same bare name.
fn collect_xctest_classes(root: Node<'_>, source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    collect_xctest_in(root, source, &[], &mut names);
    names
}

fn is_declaration_container(kind: &str) -> bool {
    matches!(
        kind,
        node::SOURCE_FILE | node::CLASS_BODY | node::ENUM_CLASS_BODY | node::PROTOCOL_BODY
    )
}

fn collect_xctest_in(
    container: Node<'_>,
    source: &str,
    path: &[String],
    out: &mut BTreeSet<String>,
) {
    if !is_declaration_container(container.kind()) {
        return;
    }
    let mut cursor = container.walk();
    for child in container.named_children(&mut cursor) {
        if child.kind() == node::CLASS_DECLARATION {
            let Some(name_node) = child.child_by_field_name(field::NAME) else {
                continue;
            };
            let bare = normalized_identifier_path(source.get(name_node.byte_range()).unwrap_or(""));
            let qualified = if path.is_empty() {
                bare.clone()
            } else {
                format!("{}.{}", path.join("."), bare)
            };
            if is_xctest_case_class(child, source) {
                out.insert(qualified);
            }
            // Only declaration bodies extend the lexical type path; executable
            // bodies are never traversed.
            let mut next = path.to_vec();
            next.push(bare);
            let mut nested = child.walk();
            for body in child.named_children(&mut nested) {
                if is_declaration_container(body.kind()) {
                    collect_xctest_in(body, source, &next, out);
                }
            }
        } else if is_declaration_container(child.kind()) {
            collect_xctest_in(child, source, path, out);
        }
    }
}

fn is_xctest_case_class(node: Node<'_>, source: &str) -> bool {
    if node
        .child_by_field_name(field::DECLARATION_KIND)
        .and_then(|kind| source.get(kind.byte_range()))
        != Some("class")
    {
        return false;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() == node::INHERITANCE_SPECIFIER)
        .any(|specifier| {
            specifier
                .child_by_field_name(field::INHERITS_FROM)
                .is_some_and(|from| {
                    let inherited = source.get(from.byte_range()).unwrap_or("");
                    last_path_segment(&normalized_identifier_path(inherited)) == "XCTestCase"
                })
        })
}

fn last_path_segment(text: &str) -> &str {
    text.trim().rsplit('.').next().unwrap_or(text)
}

/// Strip lexical backticks from an identifier for semantic tests, while the
/// canonical spelling in `canonical_text` remains untouched.
fn semantic_identifier(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('`') && trimmed.ends_with('`') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

/// Normalize a possibly qualified, backtick-escaped identifier path for
/// semantic matching: `` `Outer`.`Inner` `` and `Outer.Inner` both become
/// `Outer.Inner`. Only matching uses this form; canonical text keeps the
/// escaped spelling written in source.
fn normalized_identifier_path(text: &str) -> String {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_backtick = false;
    for character in text.trim().chars() {
        match character {
            '`' => in_backtick = !in_backtick,
            '.' if !in_backtick => segments.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    segments.push(current);
    segments
        .into_iter()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

// ---------------------------------------------------------------------------
// Container traversal and conditional reconstruction
// ---------------------------------------------------------------------------

struct BranchFrame {
    key: String,
    header: String,
    start: usize,
    /// End of the branch header directive. Also advanced past every source
    /// child (including omitted ones such as imports and diagnostics) so a
    /// branch span is never zero-length and always covers its source extent.
    end: usize,
    members: Vec<Item>,
    has_else: bool,
}

struct GroupFrame {
    key: String,
    parent: Option<String>,
    start: usize,
    branches: Vec<BranchFrame>,
}

#[allow(clippy::too_many_arguments)]
fn build_container<'t>(
    container: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    is_xctest_container: bool,
    context: &mut Context<'_>,
) -> Result<Vec<Item>, ProjectionError> {
    let mut result = Vec::new();
    let mut stack: Vec<GroupFrame> = Vec::new();
    let children: Vec<Node<'t>> = {
        let mut cursor = container.walk();
        container.children(&mut cursor).collect()
    };

    for child in children {
        // Members of the innermost open branch are owned by that branch, so
        // their keys and parent chains reflect the conditional structure.
        let (effective_parent, effective_owner) = match stack.last() {
            Some(frame) => match frame.branches.last() {
                Some(branch) => (Some(branch.key.clone()), branch.key.clone()),
                None => (parent.map(str::to_owned), owner_key.to_owned()),
            },
            None => (parent.map(str::to_owned), owner_key.to_owned()),
        };
        let kind = child.kind();
        // A branch's source span covers its last source child even when that
        // child produces no projected item (import, diagnostic, comment).
        if kind != node::DIRECTIVE
            && let Some(frame) = stack.last_mut()
            && let Some(branch) = frame.branches.last_mut()
        {
            branch.end = branch.end.max(child.end_byte());
        }
        if crate::render::is_comment(kind)
            || matches!(
                kind,
                node::DIAGNOSTIC | node::IMPORT_DECLARATION | node::PRECEDENCE_GROUP_DECLARATION
            )
        {
            continue;
        }

        if kind == node::DIRECTIVE {
            let (keyword, header, guard) = context.renderer.directive(child);
            let span = (child.start_byte(), child.end_byte());
            match DirectiveKind::from_text(&header) {
                Some(DirectiveKind::If) => {
                    let base = format!("{effective_owner}::cond::{guard}");
                    let key = context.unique(base);
                    let group_parent = effective_parent.clone();
                    let branch_key = format!("{key}::branch::0::{guard}");
                    stack.push(GroupFrame {
                        key,
                        parent: group_parent,
                        start: child.start_byte(),
                        branches: vec![BranchFrame {
                            key: branch_key,
                            header,
                            start: child.start_byte(),
                            end: child.end_byte(),
                            members: Vec::new(),
                            has_else: false,
                        }],
                    });
                }
                Some(DirectiveKind::ElseIf) | Some(DirectiveKind::Else) => {
                    let is_else = keyword == "else";
                    let Some(frame) = stack.last_mut() else {
                        return Err(cond_error(
                            context,
                            span,
                            format!("unmatched `#{keyword}` directive"),
                        ));
                    };
                    if is_else && frame.branches.iter().any(|branch| branch.has_else) {
                        return Err(cond_error(
                            context,
                            span,
                            "duplicate `#else` directive".to_owned(),
                        ));
                    }
                    if !is_else && frame.branches.iter().any(|branch| branch.has_else) {
                        return Err(cond_error(
                            context,
                            span,
                            "`#elseif` after `#else` directive".to_owned(),
                        ));
                    }
                    let index = frame.branches.len();
                    frame.branches.push(BranchFrame {
                        key: format!("{}::branch::{index}::{guard}", frame.key),
                        header,
                        start: child.start_byte(),
                        end: child.end_byte(),
                        members: Vec::new(),
                        has_else: is_else,
                    });
                }
                Some(DirectiveKind::EndIf) => {
                    let Some(frame) = stack.pop() else {
                        return Err(cond_error(
                            context,
                            span,
                            "unmatched `#endif` directive".to_owned(),
                        ));
                    };
                    let mut branches = Vec::new();
                    for branch in frame.branches {
                        let end = branch
                            .members
                            .last()
                            .map_or(branch.end, |member| member.span.end_byte())
                            .max(branch.end);
                        branches.push(Item {
                            key: branch.key,
                            parent: Some(frame.key.clone()),
                            kind: ItemKind::Module,
                            name: branch.header.clone(),
                            span: span_for_bytes(context.source, branch.start, end),
                            depth,
                            render: RenderKind::Branch {
                                header: branch.header,
                            },
                            children: branch.members,
                            modes: ModeSet::NONE,
                        });
                    }
                    let group = Item {
                        key: frame.key,
                        parent: frame.parent,
                        kind: ItemKind::Module,
                        name: format!("conditional {depth}"),
                        span: span_for_bytes(context.source, frame.start, child.end_byte()),
                        depth,
                        render: RenderKind::Group,
                        children: branches,
                        modes: ModeSet::NONE,
                    };
                    if let Some(enclosing) = stack.last_mut()
                        && let Some(branch) = enclosing.branches.last_mut()
                    {
                        branch.members.push(group);
                    } else {
                        result.push(group);
                    }
                }
                None => {
                    // A directive that is not a conditional (`#Preview`, etc.).
                    // It is not a declaration; drop it like an import.
                    continue;
                }
            }
            continue;
        }

        let built = build_decl(
            child,
            effective_parent.as_deref(),
            &effective_owner,
            depth,
            scope,
            is_xctest_container,
            context,
        )?;
        if let Some(frame) = stack.last_mut()
            && let Some(branch) = frame.branches.last_mut()
        {
            branch.members.extend(built);
            continue;
        }
        result.extend(built);
    }

    if let Some(frame) = stack.first() {
        return Err(cond_error(
            context,
            (frame.start, context.source.len()),
            "unterminated conditional compilation sequence (missing `#endif`)".to_owned(),
        ));
    }

    Ok(result)
}

fn cond_error(
    context: &Context<'_>,
    (start, end): (usize, usize),
    detail: impl Into<String>,
) -> ProjectionError {
    let _ = detail;
    ProjectionError::ErroneousSyntax {
        path: context.path.clone(),
        range: span_for_bytes(context.source, start, end),
    }
}

// ---------------------------------------------------------------------------
// Declaration building
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn build_decl<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    is_xctest_container: bool,
    context: &mut Context<'_>,
) -> Result<Vec<Item>, ProjectionError> {
    let renderer = &context.renderer;
    let attributes: Vec<Doc> = renderer
        .attributes(node)
        .into_iter()
        .map(|attribute| renderer.attribute_doc(attribute))
        .collect();

    let built = match node.kind() {
        node::CLASS_DECLARATION => {
            build_class(node, parent, owner_key, depth, scope, attributes, context)?
        }
        node::PROTOCOL_DECLARATION => {
            build_protocol(node, parent, owner_key, depth, scope, attributes, context)?
        }
        node::TYPEALIAS_DECLARATION => vec![build_leaf(
            node,
            parent,
            owner_key,
            depth,
            ItemKind::TypeAlias,
            "type_alias",
            renderer.name_text(node).unwrap_or_default(),
            attributes,
            renderer.interface(node, false),
            ModeSet::TS,
            context,
        )],
        node::ASSOCIATEDTYPE_DECLARATION => vec![build_leaf(
            node,
            parent,
            owner_key,
            depth,
            ItemKind::AssociatedType,
            "assoc_type",
            renderer.name_text(node).unwrap_or_default(),
            attributes,
            renderer.interface(node, false),
            ModeSet::TS,
            context,
        )],
        node::ENUM_ENTRY => build_variants(node, parent, owner_key, depth, attributes, context),
        node::PROPERTY_DECLARATION | node::PROTOCOL_PROPERTY_DECLARATION => {
            build_property(node, parent, owner_key, depth, scope, attributes, context)
        }
        node::FUNCTION_DECLARATION | node::PROTOCOL_FUNCTION_DECLARATION => {
            vec![build_function(
                node,
                parent,
                owner_key,
                depth,
                scope,
                is_xctest_container,
                attributes,
                context,
            )]
        }
        node::INIT_DECLARATION => vec![build_init(
            node, parent, owner_key, depth, scope, attributes, context,
        )],
        node::DEINIT_DECLARATION => vec![build_deinit(
            node, parent, owner_key, depth, scope, attributes, context,
        )],
        node::SUBSCRIPT_DECLARATION => {
            vec![build_subscript(
                node, parent, owner_key, depth, scope, attributes, context,
            )]
        }
        node::OPERATOR_DECLARATION => vec![build_operator(
            node, parent, owner_key, depth, attributes, context,
        )],
        node::MACRO_DECLARATION => vec![build_macro(
            node, parent, owner_key, depth, attributes, context,
        )],
        _ => Vec::new(),
    };
    Ok(built)
}

#[allow(clippy::too_many_arguments)]
fn build_class<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    _scope: Scope,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Result<Vec<Item>, ProjectionError> {
    let renderer = &context.renderer;
    let name = renderer.name_text(node).unwrap_or_default();
    // The extension target is the `name` field alone, not the header that
    // also carries conformances and constraints.
    let extension_target = name.clone();
    let kind_text = node
        .child_by_field_name(field::DECLARATION_KIND)
        .map(|kind| renderer.slice(kind).to_owned())
        .unwrap_or_else(|| "class".to_owned());
    let is_extension = kind_text == "extension";
    let header = renderer.interface(node, false);
    let header_text = render::render(&header, 0);

    let body = node
        .named_children(&mut node.walk())
        .find(|child| matches!(child.kind(), node::CLASS_BODY | node::ENUM_CLASS_BODY));
    let has_conformance = node
        .named_children(&mut node.walk())
        .any(|child| child.kind() == node::INHERITANCE_SPECIFIER);

    let (key, item_name, item_kind, modes) = if is_extension {
        let item_name = header_text
            .split_once("extension ")
            .map(|(_, rest)| rest.trim().to_owned())
            .unwrap_or_else(|| name.clone());
        let base = format!("{owner_key}::impl::{item_name}");
        (
            context.unique(base),
            item_name,
            ItemKind::TraitImplementation,
            if has_conformance {
                ModeSet::TS
            } else {
                ModeSet::NONE
            },
        )
    } else {
        let base = format!("{owner_key}::type::{name}");
        (context.unique(base), name, ItemKind::Type, ModeSet::TS)
    };

    // Match the extension TARGET only, never the full header with conformances
    // or a `where` clause.
    let is_xctest_container = if is_extension {
        context
            .xctest_classes
            .contains(&normalized_identifier_path(&extension_target))
    } else {
        kind_text == "class" && is_xctest_case_class(node, context.source)
    };

    let mut children = Vec::new();
    if let Some(body) = body {
        children = build_container(
            body,
            Some(&key),
            &key,
            depth + 1,
            Scope::Member,
            is_xctest_container,
            context,
        )?;
    }

    // An empty, non-conformance extension is omitted entirely.
    if is_extension && !has_conformance && children.is_empty() {
        return Ok(Vec::new());
    }

    let doc = render::with_attributes(attributes, header);
    Ok(vec![Item {
        key,
        parent: parent.map(str::to_owned),
        kind: item_kind,
        name: item_name,
        span: syntax::declaration_span(node, context.source),
        depth,
        render: RenderKind::Container(doc),
        children,
        modes,
    }])
}

fn build_protocol<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    _scope: Scope,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Result<Vec<Item>, ProjectionError> {
    let name = context.renderer.name_text(node).unwrap_or_default();
    let key = context.unique(format!("{owner_key}::trait::{name}"));
    let header = context.renderer.interface(node, false);
    let children = node
        .named_children(&mut node.walk())
        .find(|child| child.kind() == node::PROTOCOL_BODY)
        .map(|body| {
            build_container(
                body,
                Some(&key),
                &key,
                depth + 1,
                Scope::Protocol,
                false,
                context,
            )
        })
        .transpose()?
        .unwrap_or_default();

    let doc = render::with_attributes(attributes, header);
    Ok(vec![Item {
        key,
        parent: parent.map(str::to_owned),
        kind: ItemKind::Trait,
        name,
        span: syntax::declaration_span(node, context.source),
        depth,
        render: RenderKind::Container(doc),
        children,
        modes: ModeSet::TS,
    }])
}

fn build_variants<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Vec<Item> {
    // Each `case` name carries its own `data_contents` field. Pair them in
    // source order so `case a(Int), b(String)` does not reuse the first
    // payload for every variant. Raw-value initializers are never rendered.
    struct VariantDraft {
        name: String,
        data: Option<Doc>,
        start: usize,
        end: usize,
    }
    // `indirect` is an anonymous token in the grammar, so read it token-wise.
    // Non-attribute modifiers are part of the case interface and must survive.
    let case_prefix = {
        let renderer = &context.renderer;
        let mut modifiers = Vec::new();
        for child in node.children(&mut node.walk()) {
            if child.kind() == node::MODIFIERS {
                for modifier in child.children(&mut child.walk()) {
                    if modifier.kind() != node::ATTRIBUTE
                        && !crate::render::is_comment(modifier.kind())
                    {
                        modifiers.push(renderer.node_text(modifier));
                    }
                }
            } else if renderer.slice(child) == "indirect" {
                modifiers.push("indirect".to_owned());
            }
        }
        if modifiers.is_empty() {
            String::new()
        } else {
            format!("{} ", modifiers.join(" "))
        }
    };

    let mut drafts: Vec<VariantDraft> = Vec::new();
    {
        let renderer = &context.renderer;
        let mut cursor = node.walk();
        let mut children: Vec<(Option<String>, Node<'_>)> = Vec::new();
        if cursor.goto_first_child() {
            loop {
                children.push((cursor.field_name().map(str::to_owned), cursor.node()));
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        for (field_name, child) in children {
            match field_name.as_deref() {
                Some(field::NAME) if child.kind() == node::SIMPLE_IDENTIFIER => {
                    drafts.push(VariantDraft {
                        name: renderer.slice(child).to_owned(),
                        data: None,
                        start: child.start_byte(),
                        end: child.end_byte(),
                    });
                }
                Some(field::DATA_CONTENTS) => {
                    if let Some(draft) = drafts.last_mut() {
                        // Associated-value defaults are retained interface; the
                        // structural node Doc strips every nested closure.
                        draft.data = Some(renderer.node_doc(child));
                        draft.end = child.end_byte();
                    }
                }
                // `raw_value` is deliberately ignored.
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for draft in drafts {
        let key = context.unique(format!("{owner_key}::variant::{}", draft.name));
        let mut parts = vec![Doc::Text(format!("{case_prefix}case {}", draft.name))];
        parts.extend(draft.data);
        let declaration = Doc::Concat(parts);
        out.push(Item {
            key,
            parent: parent.map(str::to_owned),
            kind: ItemKind::Variant,
            name: draft.name,
            span: span_for_bytes(context.source, draft.start, draft.end),
            depth,
            render: RenderKind::Leaf(render::with_attributes(attributes.clone(), declaration)),
            children: Vec::new(),
            modes: ModeSet::TS,
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_leaf<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    kind: ItemKind,
    token: &str,
    name: String,
    attributes: Vec<Doc>,
    declaration: Doc,
    modes: ModeSet,
    context: &mut Context<'_>,
) -> Item {
    let key = context.unique(format!("{owner_key}::{token}::{name}"));
    Item {
        key,
        parent: parent.map(str::to_owned),
        kind,
        name,
        span: syntax::declaration_span(node, context.source),
        depth,
        render: RenderKind::Leaf(render::with_attributes(attributes, declaration)),
        children: Vec::new(),
        modes,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_property<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Vec<Item> {
    // A single declaration can hold several comma-separated bindings. Each
    // binding owns its own written annotation: an annotation inside a segment
    // belongs only to the names in that segment, and a trailing annotation on
    // an uninitialized run (`var x, y: Int`) is shared only across that run.
    // Initializers end a binding and never carry an annotation to the next one.
    struct BindingSegment<'t> {
        children: Vec<(Option<String>, Node<'t>)>,
        annotation: Option<Node<'t>>,
        has_initializer: bool,
    }

    let mut segments: Vec<BindingSegment<'_>> = vec![BindingSegment {
        children: Vec::new(),
        annotation: None,
        has_initializer: false,
    }];
    let mut binding_keyword = "var".to_owned();
    let mut modifier_prefix: Vec<String> = Vec::new();
    {
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                let field_name = cursor.field_name().map(str::to_owned);
                let is_separator = field_name.is_none()
                    && child.kind() != node::MODIFIERS
                    && context.renderer.slice(child) == ",";
                if is_separator {
                    segments.push(BindingSegment {
                        children: Vec::new(),
                        annotation: None,
                        has_initializer: false,
                    });
                } else {
                    match child.kind() {
                        node::VALUE_BINDING_PATTERN => {
                            binding_keyword = context.renderer.node_text(child);
                        }
                        node::MODIFIERS => {
                            let mut modifier_cursor = child.walk();
                            for modifier in child.children(&mut modifier_cursor) {
                                if modifier.kind() == node::ATTRIBUTE
                                    || crate::render::is_comment(modifier.kind())
                                {
                                    continue;
                                }
                                let text = context.renderer.node_text(modifier);
                                if !text.is_empty() {
                                    modifier_prefix.push(text);
                                }
                            }
                        }
                        _ => {}
                    }
                    if child.kind() == node::TYPE_ANNOTATION
                        && let Some(segment) = segments.last_mut()
                    {
                        segment.annotation = Some(child);
                    } else if context.renderer.slice(child) == "="
                        && let Some(segment) = segments.last_mut()
                    {
                        segment.has_initializer = true;
                    }
                    if let Some(segment) = segments.last_mut() {
                        segment.children.push((field_name, child));
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    // `var x, y: Int` shares the trailing annotation across the uninitialized
    // run that precedes it. Never cross an initializer or another annotation.
    for index in (0..segments.len()).rev() {
        let Some(annotation) = segments[index].annotation else {
            continue;
        };
        let mut previous = index;
        while previous > 0 {
            let candidate = previous - 1;
            if segments[candidate].annotation.is_some() || segments[candidate].has_initializer {
                break;
            }
            segments[candidate].annotation = Some(annotation);
            previous = candidate;
        }
    }

    // Accessor/observer interface is shared by every binding in the statement.
    let accessor = {
        let renderer = &context.renderer;
        let members: Vec<Node<'_>> = node.named_children(&mut node.walk()).collect();
        members
            .iter()
            .find(|child| {
                matches!(
                    child.kind(),
                    node::COMPUTED_PROPERTY | node::PROTOCOL_PROPERTY_REQUIREMENTS
                )
            })
            .map(|computed| format!(" {{ {} }}", renderer.accessor_specs(*computed)))
            .or_else(|| {
                members
                    .iter()
                    .find(|child| child.kind() == node::WILLSET_DIDSET_BLOCK)
                    .map(|block| format!(" {{ {} }}", renderer.accessor_specs(*block)))
            })
            .unwrap_or_default()
    };

    let declaration_start = syntax::declaration_span(node, context.source).start_byte();
    let mut out = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let annotation_doc = segment
            .annotation
            .map(|annotation| context.renderer.node_doc(annotation));
        // The first binding begins at the declaration's first real token;
        // later bindings begin at their own source pattern.
        let start = if index == 0 {
            declaration_start
        } else {
            segment
                .children
                .iter()
                .find(|(field, child)| {
                    field.as_deref() == Some(field::NAME) && child.kind() == node::PATTERN
                })
                .map_or(node.start_byte(), |(_, child)| child.start_byte())
        };
        // The span covers the complete source binding, including any
        // initializer, accessor, or observer body that canonical text strips.
        let end = segment
            .children
            .iter()
            .map(|(_, child)| child.end_byte())
            .max()
            .unwrap_or(start);
        let span = span_for_bytes(context.source, start, end.max(start));

        for (field_name, pattern) in &segment.children {
            if field_name.as_deref() != Some(field::NAME) || pattern.kind() != node::PATTERN {
                continue;
            }
            let name = binding_name(*pattern, &context.renderer);
            let mut head = modifier_prefix.clone();
            head.push(binding_keyword.clone());
            let mut head_text = render::join(&head);
            head_text.push(' ');
            head_text.push_str(&name);
            let mut parts = vec![Doc::Text(head_text)];
            parts.extend(annotation_doc.clone());
            parts.push(Doc::Text(accessor.clone()));
            let declaration = Doc::Concat(parts);

            let (kind, token, modes) = if scope == Scope::File {
                // Globals are Signatures-only regardless of an explicit type.
                (ItemKind::Value, "value", ModeSet::SIG)
            } else if annotation_doc.is_none() {
                (ItemKind::Field, "field", ModeSet::SIG)
            } else {
                (ItemKind::Field, "field", ModeSet::TS)
            };
            let key = context.unique(format!("{owner_key}::{token}::{name}"));
            out.push(Item {
                key,
                parent: parent.map(str::to_owned),
                kind,
                name,
                span,
                depth,
                render: RenderKind::Leaf(render::with_attributes(attributes.clone(), declaration)),
                children: Vec::new(),
                modes,
            });
        }
    }
    out
}

/// One source binding pattern yields exactly one item. A simple binding uses
/// its `bound_identifier`; a tuple/destructuring pattern keeps its written,
/// normalized shape (e.g. `(a, b)`) so it never collapses to an empty key and
/// no element types are inferred.
fn binding_name(pattern: Node<'_>, renderer: &Renderer<'_>) -> String {
    if let Some(identifier) = pattern.child_by_field_name("bound_identifier") {
        return renderer.slice(identifier).to_owned();
    }
    renderer.node_text(pattern)
}

#[allow(clippy::too_many_arguments)]
fn build_function<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    is_xctest_container: bool,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Item {
    let renderer = &context.renderer;
    let base = renderer.name_text(node).unwrap_or_default();
    let name = callable_name(renderer, node, &base);
    let has_test_attribute = renderer
        .attributes(node)
        .iter()
        .any(|attribute| renderer.attribute_name(*attribute) == "Test");
    let zero_params = node
        .named_children(&mut node.walk())
        .filter(|child| child.kind() == node::PARAMETER)
        .count()
        == 0;
    let static_member = node.named_children(&mut node.walk()).any(|child| {
        child.kind() == node::MODIFIERS
            && child
                .named_children(&mut child.walk())
                .any(|modifier| matches!(renderer.slice(modifier), "static" | "class"))
    });
    let semantic_name = semantic_identifier(&base);
    let is_xctest_test = is_xctest_container
        && semantic_name.starts_with("test")
        && semantic_name.len() > "test".len()
        && zero_params
        && !static_member;
    let is_test = has_test_attribute || is_xctest_test;

    let is_protocol =
        node.kind() == node::PROTOCOL_FUNCTION_DECLARATION || scope == Scope::Protocol;
    let interface = renderer.interface(node, false);
    let declaration = if is_protocol {
        interface
    } else {
        Doc::Concat(vec![interface, Doc::Text(" { ... }".to_owned())])
    };
    let (kind, token) = if scope == Scope::File {
        (ItemKind::Function, "function")
    } else {
        (ItemKind::Method, "method")
    };
    let mut modes = ModeSet::SIG;
    if is_test {
        modes = modes.with_tests();
    }
    build_leaf(
        node,
        parent,
        owner_key,
        depth,
        kind,
        token,
        name,
        attributes,
        declaration,
        modes,
        context,
    )
}

fn build_init<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Item {
    let renderer = &context.renderer;
    let mut base = "init".to_owned();
    if node
        .children(&mut node.walk())
        .any(|child| renderer.slice(child) == "?")
    {
        base.push('?');
    }
    let name = callable_name(renderer, node, &base);
    let is_protocol = scope == Scope::Protocol;
    let interface = renderer.interface(node, false);
    let declaration = if is_protocol {
        interface
    } else {
        Doc::Concat(vec![interface, Doc::Text(" { ... }".to_owned())])
    };
    build_leaf(
        node,
        parent,
        owner_key,
        depth,
        ItemKind::Constructor,
        "constructor",
        name,
        attributes,
        declaration,
        ModeSet::SIG,
        context,
    )
}

fn build_deinit<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Item {
    let _ = scope;
    // Use the written interface so attributes and modifiers are retained
    // instead of a hardcoded spelling.
    let interface = context.renderer.interface(node, false);
    let declaration = Doc::Concat(vec![interface, Doc::Text(" { ... }".to_owned())]);
    build_leaf(
        node,
        parent,
        owner_key,
        depth,
        ItemKind::Method,
        "method",
        "deinit".to_owned(),
        attributes,
        declaration,
        ModeSet::SIG,
        context,
    )
}

fn build_subscript<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    scope: Scope,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Item {
    let renderer = &context.renderer;
    let name = callable_name(renderer, node, "subscript");
    let _ = scope;
    let interface = renderer.interface(node, false);
    // Subscript accessors are interface, not executable body: preserve get/set,
    // mutation and throwing effects as specifiers in both protocols and types.
    let specs = node
        .named_children(&mut node.walk())
        .find(|child| child.kind() == node::COMPUTED_PROPERTY)
        .map(|computed| renderer.accessor_specs(computed))
        .unwrap_or_else(|| "get".to_owned());
    let declaration = Doc::Concat(vec![interface, Doc::Text(format!(" {{ {specs} }}"))]);
    build_leaf(
        node,
        parent,
        owner_key,
        depth,
        ItemKind::Method,
        "method",
        name,
        attributes,
        declaration,
        ModeSet::SIG,
        context,
    )
}

fn build_operator<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Item {
    let renderer = &context.renderer;
    let mut fixity = String::new();
    let mut operator = String::new();
    let mut precedence = String::new();
    let mut after_colon = false;
    let mut after_operator = false;
    for child in node.children(&mut node.walk()) {
        let text = renderer.slice(child);
        if text.trim().is_empty() {
            continue;
        }
        if text == ":" {
            after_colon = true;
            continue;
        }
        if after_colon {
            precedence = text.to_owned();
            continue;
        }
        if fixity.is_empty() && matches!(text, "infix" | "prefix" | "postfix") {
            fixity = text.to_owned();
            continue;
        }
        if text == "operator" {
            after_operator = true;
            continue;
        }
        if child.kind() == node::CUSTOM_OPERATOR {
            operator = text.to_owned();
        } else if after_operator && operator.is_empty() {
            // Built-in symbolic operators (`++`, `??`, `==`) can be anonymous
            // tokens with no `custom_operator` node; take the token verbatim.
            operator = text.to_owned();
        }
    }
    let mut canonical = format!("{fixity} operator {operator}");
    if !precedence.is_empty() {
        canonical.push_str(" : ");
        canonical.push_str(&precedence);
    }
    build_leaf(
        node,
        parent,
        owner_key,
        depth,
        ItemKind::Operator,
        "operator",
        operator,
        attributes,
        Doc::Text(canonical),
        ModeSet::TS,
        context,
    )
}

fn build_macro<'t>(
    node: Node<'t>,
    parent: Option<&str>,
    owner_key: &str,
    depth: usize,
    attributes: Vec<Doc>,
    context: &mut Context<'_>,
) -> Item {
    let renderer = &context.renderer;
    let base = renderer.name_text(node).unwrap_or_default();
    let name = callable_name(renderer, node, &base);
    let declaration = renderer.interface(node, false);
    build_leaf(
        node,
        parent,
        owner_key,
        depth,
        ItemKind::Function,
        "macro",
        name,
        attributes,
        declaration,
        ModeSet::SIG,
        context,
    )
}

/// Callable identity: `base(label:label:)`, including `_` labels. Same-label
/// overloads are distinguished by `KeyAllocator` ordinals.
fn callable_name(renderer: &Renderer<'_>, node: Node<'_>, base: &str) -> String {
    let mut labels = Vec::new();
    let mut in_parameters = false;
    let mut depth = 0i32;
    for child in node.children(&mut node.walk()) {
        let text = renderer.slice(child);
        if text == "(" {
            if in_parameters {
                depth += 1;
            } else {
                in_parameters = true;
                depth = 1;
            }
            continue;
        }
        if !in_parameters {
            continue;
        }
        if text == ")" {
            depth -= 1;
            if depth == 0 {
                break;
            }
            continue;
        }
        if depth != 1 || child.kind() != node::PARAMETER {
            continue;
        }
        let label = child
            .child_by_field_name(field::EXTERNAL_NAME)
            .or_else(|| child.child_by_field_name(field::NAME))
            .map(|name| renderer.slice(name).to_owned())
            .unwrap_or_default();
        labels.push(label);
    }
    if labels.is_empty() {
        format!("{base}()")
    } else {
        format!("{base}({}:)", labels.join(":"))
    }
}

// ---------------------------------------------------------------------------
// Mode filtering and materialization
// ---------------------------------------------------------------------------

fn materialize(item: &Item, mode: ProjectionMode) -> Option<Materialized> {
    match &item.render {
        RenderKind::Group => materialize_group(item, mode),
        RenderKind::Branch { .. } => None, // Branches are materialized by their group.
        RenderKind::Container(header) => {
            let children: Vec<Materialized> = item
                .children
                .iter()
                .filter_map(|child| materialize(child, mode))
                .collect();
            if !item.modes.contains(mode) && children.is_empty() {
                return None;
            }
            let docs: Vec<Doc> = children.iter().map(|child| child.doc.clone()).collect();
            let doc = render::container_doc(header.clone(), docs);
            let canonical = render::render(&doc, item.depth);
            let mut items = vec![make_item(item, canonical)];
            for child in children {
                items.extend(child.items);
            }
            Some(Materialized { doc, items })
        }
        RenderKind::Leaf(declaration) => {
            if !item.modes.contains(mode) {
                return None;
            }
            let canonical = render::render(declaration, item.depth);
            Some(Materialized {
                doc: declaration.clone(),
                items: vec![make_item(item, canonical)],
            })
        }
    }
}

fn materialize_group(item: &Item, mode: ProjectionMode) -> Option<Materialized> {
    let mut branch_data: Vec<(&Item, Vec<Materialized>)> = Vec::new();
    for branch in &item.children {
        let members: Vec<Materialized> = branch
            .children
            .iter()
            .filter_map(|member| materialize(member, mode))
            .collect();
        branch_data.push((branch, members));
    }
    if branch_data.iter().all(|(_, members)| members.is_empty()) {
        return None;
    }

    let mut branch_docs = Vec::new();
    let mut items = vec![make_item(
        item,
        String::new(), // filled in below after the group doc is built
    )];
    for (branch, members) in branch_data {
        let header = match &branch.render {
            RenderKind::Branch { header } => header.clone(),
            _ => String::new(),
        };
        let docs: Vec<Doc> = members.iter().map(|member| member.doc.clone()).collect();
        let branch_doc = if docs.is_empty() {
            Doc::Text(header.clone())
        } else {
            let mut parts = vec![Doc::Text(header), Doc::Line];
            for (index, member_doc) in docs.into_iter().enumerate() {
                if index > 0 {
                    parts.push(Doc::Line);
                }
                parts.push(member_doc);
            }
            Doc::Concat(parts)
        };
        let canonical = render::render(&branch_doc, branch.depth);
        items.push(make_item(branch, canonical));
        for member in members {
            items.extend(member.items);
        }
        branch_docs.push(branch_doc);
    }

    let mut group_parts = Vec::new();
    for (index, branch_doc) in branch_docs.into_iter().enumerate() {
        if index > 0 {
            group_parts.push(Doc::Line);
        }
        group_parts.push(branch_doc);
    }
    group_parts.push(Doc::Line);
    group_parts.push(Doc::Text("#endif".to_owned()));
    let group_doc = Doc::Concat(group_parts);
    let canonical = render::render(&group_doc, item.depth);
    items[0] = make_item(item, canonical);

    Some(Materialized {
        doc: group_doc,
        items,
    })
}

fn make_item(item: &Item, canonical_text: String) -> ProjectedItem {
    ProjectedItem {
        stable_key: item.key.clone(),
        parent_key: item.parent.clone(),
        kind: item.kind,
        name: item.name.clone(),
        span: item.span,
        canonical_text,
    }
}

#[cfg(test)]
mod tests {
    use base::{Language, LanguageProjector, ProjectionMode, RepoPath, SupportedPath};

    use super::*;

    fn project(source: &str, mode: ProjectionMode) -> String {
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let input = ProjectionInput {
            path: &path,
            source,
            mode,
        };
        let file = crate::SwiftProjector::new()
            .project(input)
            .expect("project");
        file.canonical_text().to_owned()
    }

    #[test]
    fn struct_golden() {
        let source = "struct Point: Equatable {\n    var x: Double\n    var y: Double = 0\n    static let origin: Point = Point(x: 0, y: 0)\n    func shifted(by dx: Double) -> Point { Point(x: x + dx, y: y) }\n}\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(
            types,
            "struct Point: Equatable {\n    var x: Double\n    var y: Double\n    static let origin: Point\n}\n"
        );
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "struct Point: Equatable {\n    var x: Double\n    var y: Double\n    static let origin: Point\n    func shifted(by dx: Double) -> Point { ... }\n}\n"
        );
    }

    #[test]
    fn protocol_golden() {
        let source = "protocol Repository: AnyObject, Sendable {\n    associatedtype Item: Identifiable where Item.ID == Int\n    var all: [Item] { get async throws }\n    func find(id: Int) -> Item?\n    init(seed: Int)\n}\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(
            types,
            "protocol Repository: AnyObject, Sendable {\n    associatedtype Item: Identifiable where Item.ID == Int\n    var all: [Item] { get async throws }\n}\n"
        );
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "protocol Repository: AnyObject, Sendable {\n    associatedtype Item: Identifiable where Item.ID == Int\n    var all: [Item] { get async throws }\n    func find(id: Int) -> Item?\n    init(seed: Int)\n}\n"
        );
    }

    #[test]
    fn enum_golden() {
        let source = "enum Result<T, E: Error> {\n    case success(T)\n    case failure(label: String, E)\n    case blank\n    case code = \"x\"\n    var isOK: Bool { true }\n}\n";
        let expected = "enum Result<T, E: Error> {\n    case success(T)\n    case failure(label: String, E)\n    case blank\n    case code\n    var isOK: Bool { get }\n}\n";
        assert_eq!(project(source, ProjectionMode::Types), expected);
        assert_eq!(project(source, ProjectionMode::Signatures), expected);
    }

    #[test]
    fn class_accessors_golden() {
        let source = "final class Counter: Base {\n    private(set) var value: Int = 0 { willSet { print(newValue) } didSet { print(oldValue) } }\n    var doubled: Int { get { value * 2 } set { value = newValue / 2 } }\n    var written: Int { mutating get { 0 } nonmutating set { } }\n    deinit { print(\"bye\") }\n}\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(
            types,
            "final class Counter: Base {\n    private(set) var value: Int { willSet didSet }\n    var doubled: Int { get set }\n    var written: Int { mutating get nonmutating set }\n}\n"
        );
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "final class Counter: Base {\n    private(set) var value: Int { willSet didSet }\n    var doubled: Int { get set }\n    var written: Int { mutating get nonmutating set }\n    deinit { ... }\n}\n"
        );
    }

    #[test]
    fn extension_golden() {
        let source = "extension Array: Hello where Element: Numeric {\n    func greet() -> String { \"hi\" }\n}\nextension Int {}\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(types, "extension Array: Hello where Element: Numeric {}\n");
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "extension Array: Hello where Element: Numeric {\n    func greet() -> String { ... }\n}\n"
        );
    }

    #[test]
    fn globals_macro_operator_golden() {
        let source = "typealias Handler<T> = (T) -> Void\nlet globalConst: Int = 1\nvar globalVar: String\ninfix operator ??? : AdditionPrecedence\n@attached(member, names: named(init)) macro Member() = #externalMacro(module: \"M\", type: \"T\")\nmacro M(_ x: Int) -> Int\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(
            types,
            "typealias Handler<T> = (T) -> Void\n\ninfix operator ??? : AdditionPrecedence\n"
        );
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "typealias Handler<T> = (T) -> Void\n\nlet globalConst: Int\n\nvar globalVar: String\n\ninfix operator ??? : AdditionPrecedence\n\n@attached(member, names: named(init)) macro Member()\n\nmacro M(_ x: Int) -> Int\n"
        );
    }

    #[test]
    fn attribute_closure_stripping_golden() {
        let source = "@foo(bar: { print(\"leak\") })\n@deep(bar: outer({ inner({ leak() }) }))\nstruct S {\n    func f(_ cb: () -> Void = { print(\"leak\") }, _ d: () -> Void = outer({ inner({ leak() }) })) {}\n}\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(
            types,
            "@foo(bar: { ... })\n@deep(bar: outer({ ... }))\nstruct S {}\n"
        );
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "@foo(bar: { ... })\n@deep(bar: outer({ ... }))\nstruct S {\n    func f(_ cb: () -> Void = { ... }, _ d: () -> Void = outer({ ... })) { ... }\n}\n"
        );
    }

    #[test]
    fn conditional_golden() {
        let source = "#if DEBUG\nstruct A {}\n#elseif RELEASE\n#else\nstruct B {}\n#endif\n";
        let expected = "#if DEBUG\nstruct A {}\n#elseif RELEASE\n#else\nstruct B {}\n#endif\n";
        assert_eq!(project(source, ProjectionMode::Types), expected);
        assert_eq!(project(source, ProjectionMode::Signatures), expected);
    }

    #[test]
    fn conditional_retention_golden() {
        let source = "#if DEBUG\nstruct A {}\n#else\nfunc f() {}\n#endif\n";
        let types = project(source, ProjectionMode::Types);
        assert_eq!(types, "#if DEBUG\nstruct A {}\n#else\n#endif\n");
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(
            signatures,
            "#if DEBUG\nstruct A {}\n#else\nfunc f() { ... }\n#endif\n"
        );
    }

    #[test]
    fn long_wrapping_golden() {
        let source = "func configure<Value: Codable & Sendable>(\n    _ identifier: String,\n    using decoder: JSONDecoder = JSONDecoder(),\n    forKey key: String = \"default-key\",\n    withCompletion completion: @escaping (Result<Value, Error>) -> Void\n) async throws -> Value {\n    fatalError()\n}\n";
        let expected = "func configure<Value: Codable & Sendable>(\n    _ identifier: String,\n    using decoder: JSONDecoder = JSONDecoder(),\n    forKey key: String = \"default-key\",\n    withCompletion completion: @escaping (Result<Value, Error>) -> Void,\n) async throws -> Value { ... }\n";
        assert_eq!(project(source, ProjectionMode::Signatures), expected);
    }

    #[test]
    fn tests_golden() {
        let source = "@Test func basic() { #expect(1 == 1) }\n@Suite struct MathTests {\n    @Test func add(a: Int, b: Int) { }\n}\nfinal class FooTests: XCTestCase {\n    func testThing() { }\n}\nextension FooTests {\n    func testExtension() { }\n}\n";
        let expected = "@Test func basic() { ... }\n\n@Suite struct MathTests {\n    @Test func add(a: Int, b: Int) { ... }\n}\n\nfinal class FooTests: XCTestCase {\n    func testThing() { ... }\n}\n\nextension FooTests {\n    func testExtension() { ... }\n}\n";
        assert_eq!(project(source, ProjectionMode::Tests), expected);
    }

    #[test]
    fn branch_span_covers_omitted_source_children() {
        let source =
            "#if DEBUG\nstruct A {}\n#else\nimport Foundation\n#if NEVER\n#endif\n#endif\n";
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let file = crate::SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode: ProjectionMode::Signatures,
            })
            .expect("project");
        let branches: Vec<_> = file
            .items()
            .iter()
            .filter(|item| item.kind == ItemKind::Module && item.stable_key.contains("::branch::"))
            .collect();
        assert_eq!(branches.len(), 2, "both branch headers survive: {file:?}");
        for branch in &branches {
            assert!(
                branch.span.end_byte() > branch.span.start_byte(),
                "branch span must not be empty: {branch:?}"
            );
        }
        // The `#else` branch has no projected member, but its span still ends
        // at the last source child inside the branch (the nested `#endif`,
        // not the outer one that closes the group).
        let end_of_source_children = source.find("#endif").unwrap() + "#endif".len();
        let else_branch = branches
            .iter()
            .find(|branch| branch.stable_key.contains("::branch::1::"))
            .expect("else branch");
        assert_eq!(else_branch.span.end_byte(), end_of_source_children);
    }

    #[test]
    fn xctest_detection_uses_declaration_containers_and_qualified_targets() {
        let source = "struct Holder {\n    class Inner: XCTestCase {\n        func testNested() {}\n    }\n    func build() {\n        class Local: XCTestCase {\n            func testLocal() {}\n        }\n        _ = Local()\n    }\n}\nclass Bare: XCTestCase {\n    func testBare() {}\n}\nextension Holder.Inner {\n    func testExtension() {}\n}\nextension Inner {\n    func testUnrelated() {}\n}\n";
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let project = |mode| {
            crate::SwiftProjector::new()
                .project(ProjectionInput {
                    path: &path,
                    source,
                    mode,
                })
                .expect("project")
                .canonical_text()
                .to_owned()
        };
        let tests = project(ProjectionMode::Tests);
        assert!(tests.contains("testNested"), "{tests}");
        assert!(tests.contains("testExtension"), "{tests}");
        assert!(tests.contains("testBare"), "{tests}");
        assert!(
            !tests.contains("testLocal"),
            "local classes are not containers: {tests}"
        );
        assert!(
            !tests.contains("testUnrelated"),
            "a bare name must not match a qualified class: {tests}"
        );

        // A body-only edit cannot change the Tests view.
        let edited = source.replace("class Local: XCTestCase", "class Local: Other");
        let edited_tests = crate::SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source: &edited,
                mode: ProjectionMode::Tests,
            })
            .expect("project")
            .canonical_text()
            .to_owned();
        assert_eq!(tests, edited_tests);
    }

    #[test]
    fn subscript_and_generic_where_clause_render() {
        let source = "struct Matrix {\n    subscript(row: Int, col: Int) -> Double {\n        get { 0 }\n        set { }\n    }\n}\nfunc all<T>(_ x: T) -> T where T: Equatable & Sendable { x }\n";
        let signatures = project(source, ProjectionMode::Signatures);
        assert!(
            signatures.contains("    subscript(row: Int, col: Int) -> Double { get set }"),
            "{signatures}"
        );
        assert!(
            signatures.contains("func all<T>(_ x: T) -> T where T: Equatable & Sendable { ... }"),
            "{signatures}"
        );
    }

    #[test]
    fn multi_binding_properties_expand_without_inference() {
        let source = "struct S { var x: Int = 1, y: String = \"secret\" }\n";
        assert_eq!(
            project(source, ProjectionMode::Types),
            "struct S {\n    var x: Int\n    var y: String\n}\n"
        );

        let shared = "struct S { var x, y: Int }\n";
        assert_eq!(
            project(shared, ProjectionMode::Types),
            "struct S {\n    var x: Int\n    var y: Int\n}\n"
        );

        let tuple = "struct S { let (a, b): (Int, String) = (1, \"x\") }\n";
        let signatures = project(tuple, ProjectionMode::Signatures);
        assert_eq!(signatures, "struct S {\n    let (a, b): (Int, String)\n}\n");
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let file = crate::SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source: tuple,
                mode: ProjectionMode::Signatures,
            })
            .expect("project");
        let field = file
            .items()
            .iter()
            .find(|item| item.kind == ItemKind::Field)
            .expect("tuple binding item");
        assert_eq!(field.name, "(a, b)");
        assert!(field.stable_key.contains("(a, b)"));
        assert!(!field.canonical_text.contains('='));
    }

    #[test]
    fn multi_binding_annotations_follow_their_source_binding() {
        let source = "let a: Int = 1, b: String = \"x\", c = true\n";
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(signatures, "let a: Int\n\nlet b: String\n\nlet c\n");

        let source = "let a = 1, b: String = \"x\"\n";
        let signatures = project(source, ProjectionMode::Signatures);
        assert_eq!(signatures, "let a\n\nlet b: String\n");

        // A trailing annotation is shared only across an uninitialized run.
        let shared = "var x, y: Int\n";
        let signatures = project(shared, ProjectionMode::Signatures);
        assert_eq!(signatures, "var x: Int\n\nvar y: Int\n");
    }

    #[test]
    fn property_span_covers_initializers_and_accessor_bodies() {
        let source = "struct S {\n    var v: Int { get { 1 } }\n    var w: Int = 7\n}\n";
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let file = crate::SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode: ProjectionMode::Types,
            })
            .expect("project");
        let fields: Vec<_> = file
            .items()
            .iter()
            .filter(|item| item.kind == ItemKind::Field)
            .collect();
        assert_eq!(fields.len(), 2);
        let computed = fields
            .iter()
            .find(|item| item.name == "v")
            .expect("computed field");
        let slice = &source[computed.span.start_byte()..computed.span.end_byte()];
        assert!(slice.contains("get { 1 }"), "incomplete span: {slice:?}");
        let initialized = fields
            .iter()
            .find(|item| item.name == "w")
            .expect("initialized field");
        let slice = &source[initialized.span.start_byte()..initialized.span.end_byte()];
        assert!(
            slice.contains("= 7"),
            "initializer missing from span: {slice:?}"
        );
    }

    #[test]
    fn enum_payload_defaults_are_stripped_and_indirect_is_preserved() {
        let source = "enum E {\n    case cb(action: () -> Void = { secret() })\n    indirect case next(Node)\n}\n";
        let types = project(source, ProjectionMode::Types);
        let signatures = project(source, ProjectionMode::Signatures);
        for out in [&types, &signatures] {
            assert!(!out.contains("secret"), "body leaked: {out}");
            assert!(
                out.contains("case cb(action: () -> Void = { ... })"),
                "{out}"
            );
            assert!(out.contains("indirect case next(Node)"), "{out}");
        }

        let edited = source.replace("secret()", "totallyDifferent()");
        assert_eq!(types, project(&edited, ProjectionMode::Types));
        assert_eq!(signatures, project(&edited, ProjectionMode::Signatures));
    }

    #[test]
    fn enum_cases_keep_their_own_payloads() {
        let source = "enum E { case a(Int), b(String), c }\n";
        let expected = "enum E {\n    case a(Int)\n    case b(String)\n    case c\n}\n";
        assert_eq!(project(source, ProjectionMode::Types), expected);
        assert_eq!(project(source, ProjectionMode::Signatures), expected);

        let raw = "enum E { case code = \"raw\" }\n";
        assert_eq!(
            project(raw, ProjectionMode::Types),
            "enum E {\n    case code\n}\n"
        );
    }

    #[test]
    fn escaped_qualified_xctest_paths_match_without_bare_name_leakage() {
        let source = "class Outer {\n    class Inner: XCTest.`XCTestCase` {\n        func testNested() {}\n    }\n}\nextension `Outer`.`Inner` {\n    func testExtension() {}\n}\nextension Inner {\n    func testUnrelated() {}\n}\n";
        let tests = project(source, ProjectionMode::Tests);
        assert!(tests.contains("testNested"), "{tests}");
        assert!(tests.contains("testExtension"), "{tests}");
        assert!(
            !tests.contains("testUnrelated"),
            "unrelated bare Inner must not borrow the nested class: {tests}"
        );
        assert!(
            tests.contains("`Outer`.`Inner`"),
            "escaped spelling lost: {tests}"
        );
    }

    #[test]
    fn backticked_xctest_methods_are_detected_without_losing_spelling() {
        let source = "final class FooTests: XCTestCase {\n    func `testThing`() {}\n}\n";
        let tests = project(source, ProjectionMode::Tests);
        assert!(tests.contains("func `testThing`() { ... }"), "{tests}");
    }

    #[test]
    fn declaration_spans_exclude_leading_attributes() {
        let source = "@available(*, deprecated)\nstruct Café {\n    var x: Int\n}\n";
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let file = crate::SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode: ProjectionMode::Types,
            })
            .expect("project");
        let declaration = file
            .items()
            .iter()
            .find(|item| item.kind == ItemKind::Type)
            .expect("the struct item");
        let text = &source[declaration.span.start_byte()..declaration.span.end_byte()];
        assert!(
            text.starts_with("struct Café"),
            "unexpected span slice {text:?}"
        );
        assert!(
            !text.starts_with('@'),
            "attribute leaked into span: {text:?}"
        );
        assert_eq!(declaration.span.start_line(), 1, "UTF-8 byte columns/lines");
    }

    #[test]
    fn subscript_accessor_interface_is_preserved() {
        let protocol = "protocol P {\n    subscript(i: Int) -> Int { get set }\n}\n";
        // Subscripts are callable requirements: present in Signatures, absent
        // from the type-level shape in Types.
        let types = project(protocol, ProjectionMode::Types);
        assert_eq!(types, "protocol P {}\n");
        let signatures = project(protocol, ProjectionMode::Signatures);
        assert!(
            signatures.contains("subscript(i: Int) -> Int { get set }"),
            "{signatures}"
        );

        let concrete = "struct S {\n    subscript(i: Int) -> Int {\n        get { 0 }\n        set { }\n    }\n}\n";
        let signatures = project(concrete, ProjectionMode::Signatures);
        assert!(
            signatures.contains("subscript(i: Int) -> Int { get set }"),
            "{signatures}"
        );
        assert!(!signatures.contains('0'), "{signatures}");
    }

    #[test]
    fn deinit_uses_the_written_interface() {
        let source = "final class C {\n    deinit { print(\"bye\") }\n}\n";
        let signatures = project(source, ProjectionMode::Signatures);
        assert!(signatures.contains("deinit { ... }"), "{signatures}");
        assert!(!signatures.contains("print"), "{signatures}");
    }

    #[test]
    fn anonymous_builtin_operator_names_are_preserved() {
        let source = "postfix operator ++\ninfix operator ??? : AdditionPrecedence\n";
        let types = project(source, ProjectionMode::Types);
        assert!(types.contains("postfix operator ++"), "{types}");
        assert!(
            types.contains("infix operator ??? : AdditionPrecedence"),
            "{types}"
        );
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let file = crate::SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode: ProjectionMode::Types,
            })
            .expect("project");
        assert!(
            file.items()
                .iter()
                .any(|item| item.stable_key.contains("::operator::++")),
            "{:?}",
            file.items()
        );
    }

    #[test]
    fn extension_retention_follows_mode_retained_members() {
        let source = "struct S {}\nextension S { func f() {} }\nextension S {}\n";
        assert_eq!(project(source, ProjectionMode::Types), "struct S {}\n");
        let signatures = project(source, ProjectionMode::Signatures);
        assert!(
            signatures.contains("extension S {\n    func f() { ... }\n}"),
            "{signatures}"
        );
        // A conformance header survives in Types even with no members.
        let conformance = "struct S {}\nextension S: P {}\n";
        let types = project(conformance, ProjectionMode::Types);
        assert!(types.contains("extension S: P {}"), "{types}");
    }

    #[test]
    fn a_legit_read_call_inside_a_getter_is_not_rejected() {
        let source = "struct S {\n    var v: Int {\n        get { _read(1) }\n    }\n}\nfunc _read(_ x: Int) -> Int { x }\n";
        let signatures = project(source, ProjectionMode::Signatures);
        assert!(signatures.contains("var v: Int { get }"), "{signatures}");
        assert!(
            signatures.contains("func _read(_ x: Int) -> Int { ... }"),
            "{signatures}"
        );
    }

    #[test]
    fn failable_initializer_keeps_its_label_identity() {
        let source = "struct S {\n    init?(seed: Int) { }\n}\n";
        let signatures = project(source, ProjectionMode::Signatures);
        assert!(
            signatures.contains("init?(seed: Int) { ... }"),
            "{signatures}"
        );
    }

    #[test]
    fn mode_independent_keys() {
        // A callable sibling affects ordinals in Signatures but must not change
        // keys that are shared with Types.
        let source = "struct S {\n    var x: Int\n    func f() {}\n}\n";
        let path = SupportedPath::new(RepoPath::new("src/App.swift").unwrap()).unwrap();
        let keys_for = |mode| {
            let input = ProjectionInput {
                path: &path,
                source,
                mode,
            };
            let file = crate::SwiftProjector::new()
                .project(input)
                .expect("project");
            file.items()
                .iter()
                .map(|item| item.stable_key.clone())
                .collect::<Vec<_>>()
        };
        let types = keys_for(ProjectionMode::Types);
        let signatures = keys_for(ProjectionMode::Signatures);
        for key in &types {
            assert!(
                signatures.contains(key),
                "types key {key} missing in signatures: {signatures:?}"
            );
        }
        assert_eq!(crate::SwiftProjector::new().language(), Language::Swift);
    }
}
