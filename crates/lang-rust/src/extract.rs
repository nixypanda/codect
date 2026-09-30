//! Inspection of a parsed Rust syntax tree into the shared projection model.
//!
//! Extraction follows TECHNICAL_DESIGN.md section 12: it uses the
//! repository-relative path as the file module context, preserves inline module
//! nesting, never resolves `mod name;` into another file, and never expands a
//! macro. Macro definitions and invocation output produce no items.

use base::{
    ItemKind, KeyAllocator, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput,
    ProjectionMode, SourceSpan,
};
use tree_sitter::Node;

use crate::render::{self, Doc};
use crate::syntax::{self, field, node};

/// The kind of container a declaration is nested in, which determines its
/// `ItemKind` and which product rules include it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    File,
    Module,
    Trait,
    Impl,
    Foreign,
    Fields,
    VariantFields,
}

/// A declaration node together with the outer attributes that precede it.
struct Declaration<'t> {
    node: Node<'t>,
    attributes: Vec<Node<'t>>,
}

/// A rendered declaration fragment plus the items it produces.
struct Built {
    doc: Doc,
    items: Vec<ProjectedItem>,
}

struct Context<'a> {
    source: &'a str,
    mode: ProjectionMode,
    keys: KeyAllocator,
}

impl Context<'_> {
    fn unique(&mut self, base: String) -> String {
        self.keys.unique(base)
    }
}

pub fn project(input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
    let path = input.path;
    let source = input.source;

    let mut parser = syntax::parser().map_err(|_| ProjectionError::ParseFailed {
        path: path.clone(),
        range: whole_span(source),
    })?;

    let tree =
        parser
            .parse(source.as_bytes(), None)
            .ok_or_else(|| ProjectionError::ParseFailed {
                path: path.clone(),
                range: whole_span(source),
            })?;

    let root = tree.root_node();
    if root.has_error() {
        let range = first_error_span(root).unwrap_or_else(|| whole_span(source));
        return Err(ProjectionError::ErroneousSyntax {
            path: path.clone(),
            range,
        });
    }

    let file_key = path.to_string();
    let mut context = Context {
        source,
        mode: input.mode,
        keys: KeyAllocator::new(),
    };

    let mut items = Vec::new();
    for built in build_members(root, Scope::File, &file_key, false, 0, &mut context) {
        items.extend(built.items);
    }

    ProjectedFile::try_new(path.clone(), items)
}

fn is_comment(kind: &str) -> bool {
    matches!(kind, node::LINE_COMMENT | node::BLOCK_COMMENT)
}

fn first_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).find(|child| child.is_named())
}

fn is_doc_attribute(item: Node<'_>, source: &str) -> bool {
    let attribute = match render::child_of_kind(item, node::ATTRIBUTE) {
        Some(attribute) => attribute,
        None => return false,
    };
    first_named_child(attribute)
        .and_then(|path| source.get(path.byte_range()))
        .is_some_and(|path| path == "doc")
}

fn collect_declarations<'t>(container: Node<'t>, source: &str) -> Vec<Declaration<'t>> {
    let mut declarations = Vec::new();
    let mut pending = Vec::new();
    let mut cursor = container.walk();
    for child in container.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        let kind = child.kind();
        if is_comment(kind) {
            continue;
        }
        if kind == node::ATTRIBUTE_ITEM {
            if !is_doc_attribute(child, source) {
                pending.push(child);
            }
            continue;
        }
        if kind == node::INNER_ATTRIBUTE_ITEM {
            continue;
        }
        declarations.push(Declaration {
            node: child,
            attributes: std::mem::take(&mut pending),
        });
    }
    declarations
}

fn build_members(
    container: Node<'_>,
    scope: Scope,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Vec<Built> {
    let mut members = Vec::new();
    for declaration in collect_declarations(container, context.source) {
        if let Some(built) = build_decl(&declaration, scope, container_key, nested, depth, context)
        {
            members.push(built);
        }
    }
    members
}

fn build_decl(
    declaration: &Declaration<'_>,
    scope: Scope,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Option<Built> {
    match declaration.node.kind() {
        node::STRUCT_ITEM => Some(build_struct(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        node::ENUM_ITEM => Some(build_enum(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        node::UNION_ITEM => Some(build_union(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        node::TYPE_ITEM => Some(build_type_item(
            declaration,
            scope,
            container_key,
            nested,
            depth,
            context,
        )),
        node::ASSOCIATED_TYPE => Some(build_associated_type(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        node::TRAIT_ITEM => Some(build_trait(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        node::IMPL_ITEM => build_impl(declaration, container_key, nested, depth, context),
        node::FUNCTION_ITEM | node::FUNCTION_SIGNATURE_ITEM => {
            build_signature(declaration, scope, container_key, nested, depth, context)
        }
        node::CONST_ITEM => build_constant(
            declaration,
            container_key,
            nested,
            depth,
            context,
            ItemKind::Constant,
        ),
        node::STATIC_ITEM => build_constant(
            declaration,
            container_key,
            nested,
            depth,
            context,
            ItemKind::Static,
        ),
        node::FOREIGN_MOD_ITEM => build_foreign(declaration, container_key, nested, depth, context),
        node::MOD_ITEM => build_module(declaration, container_key, nested, depth, context),
        node::FIELD_DECLARATION => Some(build_field(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        node::ENUM_VARIANT => Some(build_variant(
            declaration,
            container_key,
            nested,
            depth,
            context,
        )),
        _ => None,
    }
}

fn attribute_docs(declaration: &Declaration<'_>, context: &Context<'_>) -> Vec<Doc> {
    declaration
        .attributes
        .iter()
        .map(|attribute| render::attribute_doc(*attribute, context.source))
        .collect()
}

fn member_docs(members: &[Built]) -> Vec<Doc> {
    members.iter().map(|member| member.doc.clone()).collect()
}

fn member_items(members: Vec<Built>) -> Vec<ProjectedItem> {
    let mut items = Vec::new();
    for member in members {
        items.extend(member.items);
    }
    items
}

fn parent_key(container_key: &str, nested: bool) -> Option<String> {
    nested.then(|| container_key.to_owned())
}

#[allow(clippy::too_many_arguments)]
fn make_built(
    node: Node<'_>,
    key: String,
    parent_key: Option<String>,
    kind: ItemKind,
    name: String,
    doc: Doc,
    depth: usize,
    nested_items: Vec<ProjectedItem>,
) -> Built {
    let canonical_text = render::render(&doc, depth);
    let item = ProjectedItem {
        stable_key: key,
        parent_key,
        kind,
        name,
        span: span_of(node),
        canonical_text,
    };
    let mut items = vec![item];
    items.extend(nested_items);
    Built { doc, items }
}

fn field_name(node: Node<'_>, source: &str) -> Option<String> {
    node.child_by_field_name(field::NAME)
        .and_then(|name| source.get(name.byte_range()))
        .map(str::to_owned)
}

fn build_struct(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::type::{name}"));
    let attributes = attribute_docs(declaration, context);
    let header = render::header(
        node,
        context.source,
        &[
            node::WHERE_CLAUSE,
            node::FIELD_DECLARATION_LIST,
            node::ORDERED_FIELD_DECLARATION_LIST,
        ],
    );
    let where_clause = render::where_clause_text(node, context.source);

    match node.child_by_field_name(field::BODY) {
        None => {
            let doc = render::with_attributes(attributes, render::signature_doc(header, None));
            make_built(
                node,
                key,
                parent_key(container_key, nested),
                ItemKind::Type,
                name,
                doc,
                depth,
                Vec::new(),
            )
        }
        Some(body) if body.kind() == node::ORDERED_FIELD_DECLARATION_LIST => {
            let fields = render::render_node(body, context.source);
            let header = Doc::Concat(vec![header, Doc::Text(fields)]);
            let doc =
                render::with_attributes(attributes, render::signature_doc(header, where_clause));
            make_built(
                node,
                key,
                parent_key(container_key, nested),
                ItemKind::Type,
                name,
                doc,
                depth,
                Vec::new(),
            )
        }
        Some(body) => {
            let members = build_members(body, Scope::Fields, &key, true, depth + 1, context);
            let doc = render::with_attributes(
                attributes,
                render::container_doc(header, where_clause, member_docs(&members)),
            );
            let items = member_items(members);
            make_built(
                node,
                key,
                parent_key(container_key, nested),
                ItemKind::Type,
                name,
                doc,
                depth,
                items,
            )
        }
    }
}

fn build_union(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::type::{name}"));
    let attributes = attribute_docs(declaration, context);
    let header = render::header(
        node,
        context.source,
        &[node::WHERE_CLAUSE, node::FIELD_DECLARATION_LIST],
    );
    let where_clause = render::where_clause_text(node, context.source);
    let members = node
        .child_by_field_name(field::BODY)
        .map(|body| build_members(body, Scope::Fields, &key, true, depth + 1, context))
        .unwrap_or_default();
    let doc = render::with_attributes(
        attributes,
        render::container_doc(header, where_clause, member_docs(&members)),
    );
    let items = member_items(members);
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::Type,
        name,
        doc,
        depth,
        items,
    )
}

fn build_enum(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::type::{name}"));
    let attributes = attribute_docs(declaration, context);
    let header = render::header(
        node,
        context.source,
        &[node::WHERE_CLAUSE, node::ENUM_VARIANT_LIST],
    );
    let where_clause = render::where_clause_text(node, context.source);
    let members = node
        .child_by_field_name(field::BODY)
        .map(|body| build_members(body, Scope::VariantFields, &key, true, depth + 1, context))
        .unwrap_or_default();
    let doc = render::with_attributes(
        attributes,
        render::container_doc(header, where_clause, member_docs(&members)),
    );
    let items = member_items(members);
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::Type,
        name,
        doc,
        depth,
        items,
    )
}

fn build_field(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::field::{name}"));
    let attributes = attribute_docs(declaration, context);
    let doc = render::with_attributes(
        attributes,
        Doc::Group(Box::new(Doc::Concat(vec![
            render::field_doc(node, context.source),
            Doc::Text(",".to_owned()),
        ]))),
    );
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::Field,
        name,
        doc,
        depth,
        Vec::new(),
    )
}

fn build_variant(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::variant::{name}"));
    let attributes = attribute_docs(declaration, context);
    let doc = render::with_attributes(
        attributes,
        Doc::Group(Box::new(Doc::Concat(vec![
            render::variant_doc(node, context.source),
            Doc::Text(",".to_owned()),
        ]))),
    );
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::Variant,
        name,
        doc,
        depth,
        Vec::new(),
    )
}

fn build_type_item(
    declaration: &Declaration<'_>,
    scope: Scope,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let kind = match scope {
        Scope::Trait | Scope::Impl => ItemKind::AssociatedType,
        _ => ItemKind::TypeAlias,
    };
    let token = kind_token(kind);
    let key = context.unique(format!("{container_key}::{token}::{name}"));
    let attributes = attribute_docs(declaration, context);
    let header = render::type_alias_text(node, context.source);
    let where_clause = render::where_clause_text(node, context.source);
    let doc = render::with_attributes(attributes, render::signature_doc(header, where_clause));
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        kind,
        name,
        doc,
        depth,
        Vec::new(),
    )
}

fn build_associated_type(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::assoc_type::{name}"));
    let attributes = attribute_docs(declaration, context);
    let header = render::associated_type_text(node, context.source);
    let where_clause = render::where_clause_text(node, context.source);
    let doc = render::with_attributes(
        attributes,
        render::signature_doc(Doc::Text(header), where_clause),
    );
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::AssociatedType,
        name,
        doc,
        depth,
        Vec::new(),
    )
}

fn build_trait(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Built {
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::trait::{name}"));
    let attributes = attribute_docs(declaration, context);
    let header = render::header(
        node,
        context.source,
        &[node::WHERE_CLAUSE, node::DECLARATION_LIST],
    );
    let where_clause = render::where_clause_text(node, context.source);
    let members = render::child_of_kind(node, node::DECLARATION_LIST)
        .map(|body| build_members(body, Scope::Trait, &key, true, depth + 1, context))
        .unwrap_or_default();
    let doc = render::with_attributes(
        attributes,
        render::container_doc(header, where_clause, member_docs(&members)),
    );
    let items = member_items(members);
    make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::Trait,
        name,
        doc,
        depth,
        items,
    )
}

fn build_impl(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Option<Built> {
    let node = declaration.node;
    let trait_node = node.child_by_field_name(field::TRAIT);
    let type_node = node.child_by_field_name(field::TYPE)?;
    let type_name = render::render_node(type_node, context.source);
    let (key, name) = match trait_node {
        Some(trait_node) => {
            let trait_name = render::render_node(trait_node, context.source);
            (
                format!("impl {trait_name} for {type_name}"),
                format!("{trait_name} for {type_name}"),
            )
        }
        None => {
            let key = format!("impl {type_name}");
            (key.clone(), type_name)
        }
    };
    let key = context.unique(key);
    let attributes = attribute_docs(declaration, context);
    let header = render::header(
        node,
        context.source,
        &[node::WHERE_CLAUSE, node::DECLARATION_LIST],
    );
    let where_clause = render::where_clause_text(node, context.source);
    let members = render::child_of_kind(node, node::DECLARATION_LIST)
        .map(|body| build_members(body, Scope::Impl, &key, true, depth + 1, context))
        .unwrap_or_default();

    // A trait implementation header is a type relationship and survives Types
    // mode even when it has no associated type. An inherent implementation with
    // nothing included in the current mode is dropped entirely (section 12.2).
    if trait_node.is_none() && members.is_empty() {
        return None;
    }

    let doc = render::with_attributes(
        attributes,
        render::container_doc(header, where_clause, member_docs(&members)),
    );
    let items = member_items(members);
    Some(make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::TraitImplementation,
        name,
        doc,
        depth,
        items,
    ))
}

fn build_signature(
    declaration: &Declaration<'_>,
    scope: Scope,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Option<Built> {
    if context.mode != ProjectionMode::Signatures {
        return None;
    }
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let kind = match scope {
        Scope::Trait | Scope::Impl => ItemKind::Method,
        _ => ItemKind::Function,
    };
    let key = context.unique(format!("{container_key}::{}::{name}", kind_token(kind)));
    let attributes = attribute_docs(declaration, context);
    let header = render::header(node, context.source, &[node::WHERE_CLAUSE, node::BLOCK]);
    let where_clause = render::where_clause_text(node, context.source);
    let doc = render::with_attributes(attributes, render::signature_doc(header, where_clause));
    Some(make_built(
        node,
        key,
        parent_key(container_key, nested),
        kind,
        name,
        doc,
        depth,
        Vec::new(),
    ))
}

fn build_constant(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
    kind: ItemKind,
) -> Option<Built> {
    if context.mode != ProjectionMode::Signatures {
        return None;
    }
    let node = declaration.node;
    let name = field_name(node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::{}::{name}", kind_token(kind)));
    let attributes = attribute_docs(declaration, context);
    let text = render::constant_text(node, context.source);
    let doc = render::with_attributes(attributes, Doc::Text(format!("{text};")));
    Some(make_built(
        node,
        key,
        parent_key(container_key, nested),
        kind,
        name,
        doc,
        depth,
        Vec::new(),
    ))
}

fn build_foreign(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Option<Built> {
    if context.mode != ProjectionMode::Signatures {
        return None;
    }
    let node = declaration.node;
    let body = node.child_by_field_name(field::BODY)?;
    let name = render::child_of_kind(node, node::EXTERN_MODIFIER)
        .map(|modifier| render::render_node(modifier, context.source))
        .unwrap_or_else(|| "extern".to_owned());
    let key = context.unique(format!("{container_key}::extern::{name}"));
    let attributes = attribute_docs(declaration, context);
    let members = build_members(body, Scope::Foreign, &key, true, depth + 1, context);
    if members.is_empty() {
        return None;
    }
    let header = render::header(node, context.source, &[node::DECLARATION_LIST]);
    let doc = render::with_attributes(
        attributes,
        render::container_doc(header, None, member_docs(&members)),
    );
    let items = member_items(members);
    Some(make_built(
        node,
        key,
        parent_key(container_key, nested),
        ItemKind::ForeignBlock,
        name,
        doc,
        depth,
        items,
    ))
}

fn build_module(
    declaration: &Declaration<'_>,
    container_key: &str,
    nested: bool,
    depth: usize,
    context: &mut Context<'_>,
) -> Option<Built> {
    // An out-of-line `mod name;` has no content here; the referenced file is
    // projected independently by its own path (section 12.1).
    let body = declaration.node.child_by_field_name(field::BODY)?;
    let name = field_name(declaration.node, context.source).unwrap_or_default();
    let key = context.unique(format!("{container_key}::mod::{name}"));
    let attributes = attribute_docs(declaration, context);
    let members = build_members(body, Scope::Module, &key, true, depth + 1, context);
    if members.is_empty() {
        return None;
    }
    let header = render::header(declaration.node, context.source, &[node::DECLARATION_LIST]);
    let doc = render::with_attributes(
        attributes,
        render::container_doc(header, None, member_docs(&members)),
    );
    let items = member_items(members);
    Some(make_built(
        declaration.node,
        key,
        parent_key(container_key, nested),
        ItemKind::Module,
        name,
        doc,
        depth,
        items,
    ))
}

fn kind_token(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Module => "mod",
        ItemKind::Type => "type",
        ItemKind::TypeAlias => "alias",
        ItemKind::Constructor => "constructor",
        ItemKind::Field => "field",
        ItemKind::Variant => "variant",
        ItemKind::Trait => "trait",
        ItemKind::TraitImplementation => "impl",
        ItemKind::AssociatedType => "assoc_type",
        ItemKind::TypeFamily => "type_family",
        ItemKind::PatternSynonym => "pattern",
        ItemKind::Function => "fn",
        ItemKind::Method => "method",
        ItemKind::Value => "value",
        ItemKind::Constant => "const",
        ItemKind::Static => "static",
        ItemKind::Port => "port",
        ItemKind::Operator => "operator",
        ItemKind::ForeignBlock => "extern",
    }
}

fn span_of(node: Node<'_>) -> SourceSpan {
    let start = node.start_position();
    let end = node.end_position();
    SourceSpan::new(
        node.start_byte(),
        node.end_byte(),
        start.row,
        start.column,
        end.row,
        end.column,
    )
}

fn whole_span(source: &str) -> SourceSpan {
    let bytes = source.as_bytes();
    let mut line = 0;
    let mut column = 0;
    for &byte in bytes {
        if byte == b'\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    SourceSpan::new(0, source.len(), 0, 0, line, column)
}

/// The first `ERROR` or missing node in source order, used for the diagnostic
/// range. The traversal is iterative so deeply nested expressions cannot
/// overflow the stack (section 18).
fn first_error_span(root: Node<'_>) -> Option<SourceSpan> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            return Some(span_of(node));
        }
        let mut children: Vec<Node<'_>> = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            children.push(child);
        }
        for child in children.into_iter().rev() {
            stack.push(child);
        }
    }
    None
}
