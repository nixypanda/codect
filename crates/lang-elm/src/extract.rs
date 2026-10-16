//! Inspection of a parsed Elm syntax tree into the shared projection model.
//!
//! Only direct children of the file node are considered, which is what excludes
//! imports and declarations nested inside expressions (`let`, `case`, and
//! function bodies) without inspecting any expression. An explicit annotation
//! is paired with its declaration by declared name rather than by adjacency or
//! order, and Tests mode retains nothing because a test's name is a string
//! literal inside a body that is never inspected.

use std::collections::HashMap;

use base::{
    ItemKind, KeyAllocator, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput,
    ProjectionMode, SourceSpan,
};
use tree_sitter::Node;

use crate::render::Renderer;
use crate::syntax::{self, field, node};

pub(crate) fn project_file(input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
    let tree = syntax::parse(input.source, input.path)?;
    let root = tree.root_node();

    match input.mode {
        ProjectionMode::Types | ProjectionMode::Signatures => {}
        ProjectionMode::Tests => {
            return ProjectedFile::try_new(input.path.clone(), Vec::new());
        }
    }

    let renderer = Renderer::new(input.path, input.source);

    let annotations = collect_annotations(&renderer, root)?;
    let mut builder = Builder {
        module: module_name(&renderer, root)?,
        mode: input.mode,
        items: Vec::new(),
        keys: KeyAllocator::new(),
    };

    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            node::MODULE_DECLARATION => {
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
            node::TYPE_DECLARATION => {
                let key = builder.push(
                    ItemKind::Type,
                    renderer.field_name(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.type_declaration(child)?.render(),
                    None,
                );
                let mut variants = child.walk();
                for variant in child.children_by_field_name(field::UNION_VARIANT, &mut variants) {
                    builder.push(
                        ItemKind::Constructor,
                        renderer.field_name(variant)?.to_owned(),
                        syntax::node_span(variant),
                        renderer.variant_body(variant)?.render(),
                        Some(key.clone()),
                    );
                }
            }
            node::TYPE_ALIAS_DECLARATION => {
                let key = builder.push(
                    ItemKind::TypeAlias,
                    renderer.field_name(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.type_alias(child)?.render(),
                    None,
                );
                let mut fields = child.walk();
                for field in child.children_by_field_name(field::FIELD_TYPE, &mut fields) {
                    builder.push(
                        ItemKind::Field,
                        renderer.field_name(field)?.to_owned(),
                        syntax::node_span(field),
                        renderer.field_type(field)?.render(),
                        Some(key.clone()),
                    );
                }
            }
            node::VALUE_DECLARATION if builder.mode == ProjectionMode::Signatures => {
                if let Some(left) = child.child_by_field_name(field::FUNCTION_LEFT) {
                    let name = renderer.declaration_name(left)?.to_owned();
                    let text = match annotations.get(&name) {
                        Some(annotation) => annotation.clone(),
                        None => renderer.missing_annotation(&name).render(),
                    };
                    let mut patterns = left.walk();
                    let kind = if left
                        .children_by_field_name(field::PATTERN, &mut patterns)
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
            node::PORT_ANNOTATION if builder.mode == ProjectionMode::Signatures => {
                builder.push(
                    ItemKind::Port,
                    renderer.field_name(child)?.to_owned(),
                    syntax::node_span(child),
                    renderer.type_annotation(child, true)?.render(),
                    None,
                );
            }
            node::INFIX_DECLARATION if builder.mode == ProjectionMode::Signatures => {
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
        .find(|child| child.kind() == node::MODULE_DECLARATION)
    {
        Some(module) => Ok(renderer.module_name_and_port(module)?.0),
        None => Ok(String::new()),
    }
}

fn collect_annotations(
    renderer: &Renderer<'_>,
    root: Node<'_>,
) -> Result<HashMap<String, String>, ProjectionError> {
    let mut annotations = HashMap::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if child.kind() == node::TYPE_ANNOTATION {
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
    keys: KeyAllocator,
}

impl Builder {
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
        self.keys.unique(base)
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
