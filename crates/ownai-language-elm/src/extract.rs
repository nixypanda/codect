//! Inspection of a parsed Elm syntax tree into the shared projection model.
//!
//! Only direct children of the file node are considered, which is what excludes
//! imports and declarations nested inside expressions (`let`, `case`, and
//! function bodies) without inspecting any expression.

use std::collections::HashMap;

use ownai_core::{
    ItemKind, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput, ProjectionMode,
    SourceSpan,
};
use tree_sitter::Node;

use crate::render::Renderer;
use crate::syntax::{
    self, FIELD_FIELD_TYPE, FIELD_FUNCTION_LEFT, FIELD_PATTERN, FIELD_UNION_VARIANT,
    INFIX_DECLARATION, MODULE_DECLARATION, PORT_ANNOTATION, TYPE_ALIAS_DECLARATION,
    TYPE_ANNOTATION, TYPE_DECLARATION, VALUE_DECLARATION,
};

pub(crate) fn project_file(input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
    let tree = syntax::parse(input.source, input.path)?;
    let root = tree.root_node();
    let renderer = Renderer::new(input.path, input.source);

    let annotations = collect_annotations(&renderer, root)?;
    let mut builder = Builder {
        module: module_name(&renderer, root)?,
        mode: input.mode,
        items: Vec::new(),
        keys: HashMap::new(),
    };

    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            MODULE_DECLARATION => {
                let (name, is_port) = renderer.module_name_and_port(child)?;
                let canonical_text = renderer.module(child, is_port)?.render();
                builder.push(
                    ItemKind::Module,
                    name,
                    syntax::node_span(child),
                    canonical_text,
                    None,
                );
            }
            TYPE_DECLARATION => {
                let key = builder.push(
                    ItemKind::Type,
                    renderer.field_name(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.type_declaration(child)?.render(),
                    None,
                );
                let mut variants = child.walk();
                for variant in child.children_by_field_name(FIELD_UNION_VARIANT, &mut variants) {
                    builder.push(
                        ItemKind::Constructor,
                        renderer.field_name(variant)?.to_owned(),
                        syntax::node_span(variant),
                        renderer.variant_body(variant)?.render(),
                        Some(key.clone()),
                    );
                }
            }
            TYPE_ALIAS_DECLARATION => {
                let key = builder.push(
                    ItemKind::TypeAlias,
                    renderer.field_name(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.type_alias(child)?.render(),
                    None,
                );
                let mut fields = child.walk();
                for field in child.children_by_field_name(FIELD_FIELD_TYPE, &mut fields) {
                    builder.push(
                        ItemKind::Field,
                        renderer.field_name(field)?.to_owned(),
                        syntax::node_span(field),
                        renderer.field_type(field)?.render(),
                        Some(key.clone()),
                    );
                }
            }
            VALUE_DECLARATION if builder.mode == ProjectionMode::Signatures => {
                // A destructuring declaration has no declared lower-case name
                // to key or render, so it is not part of the named surface.
                if let Some(left) = child.child_by_field_name(FIELD_FUNCTION_LEFT) {
                    let name = renderer.declaration_name(left)?.to_owned();
                    let text = match annotations.get(&name) {
                        Some(annotation) => annotation.clone(),
                        None => renderer.missing_annotation(&name).render(),
                    };
                    let mut patterns = left.walk();
                    let kind = if left
                        .children_by_field_name(FIELD_PATTERN, &mut patterns)
                        .next()
                        .is_some()
                    {
                        ItemKind::Function
                    } else {
                        ItemKind::Value
                    };
                    builder.push(kind, name, syntax::node_span(child), text, None);
                }
            }
            PORT_ANNOTATION if builder.mode == ProjectionMode::Signatures => {
                builder.push(
                    ItemKind::Port,
                    renderer.field_name(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.type_annotation(child, true)?.render(),
                    None,
                );
            }
            INFIX_DECLARATION if builder.mode == ProjectionMode::Signatures => {
                builder.push(
                    ItemKind::Operator,
                    renderer.operator(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.infix(child)?.render(),
                    None,
                );
            }
            _ => {}
        }
    }

    ProjectedFile::try_new(input.path.clone(), builder.items)
}

fn module_name(renderer: &Renderer<'_>, root: Node<'_>) -> Result<String, ProjectionError> {
    let mut cursor = root.walk();
    match root
        .named_children(&mut cursor)
        .find(|child| child.kind() == MODULE_DECLARATION)
    {
        Some(module) => Ok(renderer.module_name_and_port(module)?.0),
        None => Ok(String::new()),
    }
}

/// Explicit annotations are indexed by declared name so that pairing does not
/// depend on adjacency or on a fixed relative order (TECHNICAL_DESIGN.md 11.3).
fn collect_annotations(
    renderer: &Renderer<'_>,
    root: Node<'_>,
) -> Result<HashMap<String, String>, ProjectionError> {
    let mut annotations = HashMap::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == TYPE_ANNOTATION {
            let name = renderer.field_name(child)?.to_owned();
            let text = renderer.type_annotation(child, false)?.render();
            annotations.entry(name).or_insert(text);
        }
    }
    Ok(annotations)
}

struct Builder {
    module: String,
    mode: ProjectionMode,
    items: Vec<ProjectedItem>,
    keys: HashMap<String, usize>,
}

impl Builder {
    /// Returns the item's stable key so callers can mark nested members with
    /// their owner. Collisions get a deterministic source-order ordinal and
    /// never a byte offset (TECHNICAL_DESIGN.md 5.2).
    fn push(
        &mut self,
        kind: ItemKind,
        name: String,
        span: SourceSpan,
        canonical_text: String,
        parent_key: Option<String>,
    ) -> String {
        let stable_key = self.unique_key(kind, &name);
        self.items.push(ProjectedItem {
            stable_key: stable_key.clone(),
            parent_key,
            kind,
            name,
            span,
            canonical_text,
        });
        stable_key
    }

    fn unique_key(&mut self, kind: ItemKind, name: &str) -> String {
        let base = if kind == ItemKind::Module {
            self.module.clone()
        } else if self.module.is_empty() {
            format!("{} {name}", kind_label(kind))
        } else {
            format!("{} {} {name}", self.module, kind_label(kind))
        };
        let ordinal = self.keys.entry(base.clone()).or_insert(0);
        *ordinal += 1;
        if *ordinal == 1 {
            base
        } else {
            format!("{base}#{ordinal}")
        }
    }
}

fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Module => "module",
        ItemKind::Type => "type",
        ItemKind::TypeAlias => "type alias",
        ItemKind::Constructor => "constructor",
        ItemKind::Field => "field",
        ItemKind::Function => "function",
        ItemKind::Value => "value",
        ItemKind::Port => "port",
        ItemKind::Operator => "operator",
        _ => "item",
    }
}
