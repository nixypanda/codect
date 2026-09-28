//! Inspection of a parsed Python syntax tree into the shared projection model.
//!
//! Extraction follows the Python projection rules: the repository-relative path
//! is the file module context, only module- and class-level declarations are
//! considered, and function and method bodies are never inspected. Decorators
//! are preserved because they can change a declaration's meaning, while
//! docstrings, comments, imports, and initializers are omitted.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use ownai_core::{
    ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput,
    ProjectionMode, SourceSpan,
};
use tree_sitter::Node;

use crate::render::{self, Doc, Renderer};
use crate::syntax::{
    self, ASSIGNMENT, ATTRIBUTE, CALL, CLASS_DEFINITION, COMMENT, DECORATED_DEFINITION, DECORATOR,
    EXPRESSION_STATEMENT, FIELD_BODY, FIELD_DEFINITION, FIELD_LEFT, FIELD_NAME, FIELD_RETURN_TYPE,
    FIELD_RIGHT, FIELD_SUPERCLASSES, FIELD_TYPE, FIELD_TYPE_PARAMETERS, FUNCTION_DEFINITION,
    IDENTIFIER, PASS_STATEMENT, STRING, TYPE_ALIAS_STATEMENT,
};

/// Whether a declaration sits at module level or inside a class body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    Module,
    Class { enum_like: bool },
}

/// A rendered declaration fragment plus the items it produces.
struct Built {
    doc: Doc,
    items: Vec<ProjectedItem>,
}

struct Builder<'a> {
    renderer: &'a Renderer<'a>,
    mode: ProjectionMode,
    items: Vec<ProjectedItem>,
    keys: HashMap<String, usize>,
}

pub(crate) fn project_file(input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
    let tree = syntax::parse(input.source, input.path)?;
    let root = tree.root_node();
    let renderer = Renderer::new(input.path, input.source);

    let mut builder = Builder {
        renderer: &renderer,
        mode: input.mode,
        items: Vec::new(),
        keys: HashMap::new(),
    };

    let file_key = input.path.to_string();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if let Some(built) = builder.member(child, &file_key, Scope::Module, 0)? {
            builder.items.extend(built.items);
        }
    }

    Ok(ProjectedFile::new(
        input.path.clone(),
        Language::Python,
        builder.items,
    ))
}

impl Builder<'_> {
    /// Stable keys are unique inside a projected file; collisions take a
    /// deterministic source-order ordinal instead of a byte offset.
    fn unique(&mut self, base: String) -> String {
        match self.keys.entry(base.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(1);
                base
            }
            Entry::Occupied(mut entry) => {
                let ordinal = entry.get_mut();
                let key = format!("{base}~{ordinal}");
                *ordinal += 1;
                key
            }
        }
    }

    fn member(
        &mut self,
        node: Node<'_>,
        container_key: &str,
        scope: Scope,
        depth: usize,
    ) -> Result<Option<Built>, ProjectionError> {
        match node.kind() {
            COMMENT | PASS_STATEMENT => Ok(None),
            DECORATED_DEFINITION => {
                let decorators = self.decorators(node);
                let definition = node
                    .child_by_field_name(FIELD_DEFINITION)
                    .ok_or_else(|| self.missing(node, "decorated definition has no definition"))?;
                match definition.kind() {
                    CLASS_DEFINITION => Ok(Some(self.class_decl(
                        definition,
                        decorators,
                        container_key,
                        scope,
                        depth,
                    )?)),
                    FUNCTION_DEFINITION => {
                        self.function_decl(definition, decorators, container_key, scope, depth)
                    }
                    _ => self.invariant(node, "unsupported decorated definition"),
                }
            }
            CLASS_DEFINITION => Ok(Some(self.class_decl(
                node,
                Vec::new(),
                container_key,
                scope,
                depth,
            )?)),
            FUNCTION_DEFINITION => {
                self.function_decl(node, Vec::new(), container_key, scope, depth)
            }
            TYPE_ALIAS_STATEMENT => Ok(Some(self.type_alias(node, container_key, scope, depth)?)),
            EXPRESSION_STATEMENT => self.expression_statement(node, container_key, scope, depth),
            _ => Ok(None),
        }
    }

    fn invariant<T>(
        &self,
        node: Node<'_>,
        detail: impl Into<String>,
    ) -> Result<T, ProjectionError> {
        self.renderer.invariant(node, detail)
    }

    fn missing(&self, node: Node<'_>, detail: impl Into<String>) -> ProjectionError {
        ProjectionError::AstInvariant {
            path: self.renderer.path().clone(),
            language: Language::Python,
            range: syntax::node_span(node),
            detail: detail.into(),
        }
    }

    fn decorators(&self, definition: Node<'_>) -> Vec<Doc> {
        let mut docs = Vec::new();
        let mut cursor = definition.walk();
        for child in definition.children(&mut cursor) {
            if child.kind() == DECORATOR {
                docs.push(Doc::Text(self.renderer.node_text(child)));
            }
        }
        docs
    }

    fn class_decl(
        &mut self,
        node: Node<'_>,
        decorators: Vec<Doc>,
        container_key: &str,
        outer: Scope,
        depth: usize,
    ) -> Result<Built, ProjectionError> {
        let name = self
            .renderer
            .field_text(node, FIELD_NAME)
            .ok_or_else(|| self.missing(node, "class has no name"))?
            .to_owned();
        let key = self.unique(format!("{container_key}::class::{name}"));
        let enum_like = is_enum_like(self.renderer, node);

        let mut header = vec![Doc::Text(format!("class {name}"))];
        let type_parameters = node.child_by_field_name(FIELD_TYPE_PARAMETERS);
        let superclasses = node.child_by_field_name(FIELD_SUPERCLASSES);
        if let Some(parameters) = type_parameters {
            let list = self.renderer.bracket_list(parameters, "[", "]");
            // The last list is the primary break point; an earlier list stays
            // grouped so it remains inline when it fits on its own.
            header.push(if superclasses.is_some() {
                Doc::Group(Box::new(list))
            } else {
                list
            });
        }
        if let Some(superclasses) = superclasses {
            header.push(self.renderer.bracket_list(superclasses, "(", ")"));
        }
        header.push(Doc::Text(":".to_owned()));
        let header = Doc::Group(Box::new(Doc::Concat(header)));

        let members = match node.child_by_field_name(FIELD_BODY) {
            Some(body) => self.members(body, &key, Scope::Class { enum_like }, depth + 1)?,
            None => Vec::new(),
        };
        let doc = container_doc(decorators, header, member_docs(&members));
        let parent = nested_parent(container_key, outer);
        let items = member_items(members);
        Ok(make_built(
            node,
            key,
            parent,
            ItemKind::Type,
            name,
            doc,
            depth,
            items,
        ))
    }

    fn function_decl(
        &mut self,
        node: Node<'_>,
        decorators: Vec<Doc>,
        container_key: &str,
        scope: Scope,
        depth: usize,
    ) -> Result<Option<Built>, ProjectionError> {
        if self.mode != ProjectionMode::Signatures {
            return Ok(None);
        }
        let name = self
            .renderer
            .field_text(node, FIELD_NAME)
            .ok_or_else(|| self.missing(node, "function has no name"))?
            .to_owned();
        let is_method = matches!(scope, Scope::Class { .. });
        let kind_tag = if is_method { "method" } else { "fn" };
        let key = self.unique(format!("{container_key}::{kind_tag}::{name}"));

        let mut header = Vec::new();
        if self.renderer.has_child_of_kind(node, "async") {
            header.push(Doc::Text("async ".to_owned()));
        }
        header.push(Doc::Text(format!("def {name}")));
        let type_parameters = node.child_by_field_name(FIELD_TYPE_PARAMETERS);
        let parameters = node.child_by_field_name("parameters");
        if let Some(type_parameters) = type_parameters {
            let list = self.renderer.bracket_list(type_parameters, "[", "]");
            // The parameter list is the primary break point; type parameters
            // stay grouped so they remain inline when they fit on their own.
            header.push(if parameters.is_some() {
                Doc::Group(Box::new(list))
            } else {
                list
            });
        }
        match parameters {
            Some(parameters) => header.push(self.renderer.bracket_list(parameters, "(", ")")),
            None => self.invariant(node, "function has no parameter list")?,
        }
        if let Some(return_type) = node.child_by_field_name(FIELD_RETURN_TYPE) {
            header.push(Doc::Text(" -> ".to_owned()));
            header.push(Doc::Text(self.renderer.node_text(return_type)));
        }
        header.push(Doc::Text(": ...".to_owned()));

        // Group the whole header so the parameter list breaks when the header,
        // including the return type, does not fit (not just the list alone).
        let doc = signature_doc(decorators, Doc::Group(Box::new(Doc::Concat(header))));
        let parent = nested_parent(container_key, scope);
        let item_kind = if is_method {
            ItemKind::Method
        } else {
            ItemKind::Function
        };
        Ok(Some(make_built(
            node,
            key,
            parent,
            item_kind,
            name,
            doc,
            depth,
            Vec::new(),
        )))
    }

    fn type_alias(
        &mut self,
        node: Node<'_>,
        container_key: &str,
        scope: Scope,
        depth: usize,
    ) -> Result<Built, ProjectionError> {
        let left = node
            .child_by_field_name(FIELD_LEFT)
            .ok_or_else(|| self.missing(node, "type alias has no left side"))?;
        let name = self.renderer.node_text(left);
        let key = self.unique(format!("{container_key}::alias::{name}"));
        let mut text = format!("type {name}");
        if let Some(right) = node.child_by_field_name(FIELD_RIGHT) {
            text.push_str(" = ");
            text.push_str(&self.renderer.node_text(right));
        }
        Ok(make_built(
            node,
            key,
            parent_key_for(container_key, scope),
            ItemKind::TypeAlias,
            name,
            Doc::Text(text),
            depth,
            Vec::new(),
        ))
    }

    fn members(
        &mut self,
        body: Node<'_>,
        container_key: &str,
        scope: Scope,
        depth: usize,
    ) -> Result<Vec<Built>, ProjectionError> {
        let mut members = Vec::new();
        let mut cursor = body.walk();
        for child in body.named_children(&mut cursor) {
            if let Some(built) = self.member(child, container_key, scope, depth)? {
                members.push(built);
            }
        }
        Ok(members)
    }

    fn expression_statement(
        &mut self,
        node: Node<'_>,
        container_key: &str,
        scope: Scope,
        depth: usize,
    ) -> Result<Option<Built>, ProjectionError> {
        let mut cursor = node.walk();
        let named: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
        if named.len() == 1 && named[0].kind() == STRING {
            // A module- or class-level docstring.
            return Ok(None);
        }
        for child in named {
            if child.kind() == ASSIGNMENT {
                return self.assignment(child, container_key, scope, depth);
            }
        }
        Ok(None)
    }

    fn assignment(
        &mut self,
        node: Node<'_>,
        container_key: &str,
        scope: Scope,
        depth: usize,
    ) -> Result<Option<Built>, ProjectionError> {
        let Some(left) = node.child_by_field_name(FIELD_LEFT) else {
            return Ok(None);
        };
        let left_text = self.renderer.node_text(left);
        let type_annotation = node.child_by_field_name(FIELD_TYPE);

        if let Some(annotation) = type_annotation {
            let text = format!("{left_text}: {}", self.renderer.node_text(annotation));
            let in_class = matches!(scope, Scope::Class { .. });
            let (kind, tag) = if in_class {
                (ItemKind::Field, "field")
            } else {
                (ItemKind::Value, "value")
            };
            // A module-level value with a declared type is a signature, not a
            // type declaration.
            if !in_class && self.mode == ProjectionMode::Types {
                return Ok(None);
            }
            let key = self.unique(format!("{container_key}::{tag}::{left_text}"));
            return Ok(Some(make_built(
                node,
                key,
                parent_key_for(container_key, scope),
                kind,
                left_text,
                Doc::Text(text),
                depth,
                Vec::new(),
            )));
        }

        if self.is_type_constructor(node.child_by_field_name(FIELD_RIGHT)) {
            let text = self.renderer.node_text(node);
            let key = self.unique(format!("{container_key}::type::{left_text}"));
            return Ok(Some(make_built(
                node,
                key,
                parent_key_for(container_key, scope),
                ItemKind::Type,
                left_text,
                Doc::Text(text),
                depth,
                Vec::new(),
            )));
        }

        // A bare assignment has no declared type. An enum member defines the
        // type's shape and appears in both modes; any other value is a
        // signature and appears only in Signatures mode.
        let enum_member = matches!(scope, Scope::Class { enum_like: true });
        if !enum_member && self.mode == ProjectionMode::Types {
            return Ok(None);
        }
        let (kind, tag) = if enum_member {
            (ItemKind::Variant, "variant")
        } else {
            (ItemKind::Value, "value")
        };
        let key = self.unique(format!("{container_key}::{tag}::{left_text}"));
        Ok(Some(make_built(
            node,
            key,
            parent_key_for(container_key, scope),
            kind,
            left_text.clone(),
            Doc::Text(left_text),
            depth,
            Vec::new(),
        )))
    }

    fn is_type_constructor(&self, right: Option<Node<'_>>) -> bool {
        let Some(right) = right else {
            return false;
        };
        if right.kind() != CALL {
            return false;
        }
        let Some(function) = right.child_by_field_name("function") else {
            return false;
        };
        let name = match function.kind() {
            IDENTIFIER => function,
            ATTRIBUTE => match function.child_by_field_name("attribute") {
                Some(attribute) => attribute,
                None => return false,
            },
            _ => return false,
        };
        matches!(
            self.renderer.slice(name),
            "TypeVar" | "ParamSpec" | "TypeVarTuple" | "NewType"
        )
    }
}

fn is_enum_like(renderer: &Renderer<'_>, class: Node<'_>) -> bool {
    let Some(superclasses) = class.child_by_field_name(FIELD_SUPERCLASSES) else {
        return false;
    };
    let mut cursor = superclasses.walk();
    for child in superclasses.named_children(&mut cursor) {
        let name = match child.kind() {
            IDENTIFIER => Some(renderer.slice(child)),
            ATTRIBUTE => child
                .child_by_field_name("attribute")
                .map(|attribute| renderer.slice(attribute)),
            _ => None,
        };
        if matches!(
            name,
            Some("Enum" | "IntEnum" | "StrEnum" | "Flag" | "IntFlag" | "EnumMeta")
        ) {
            return true;
        }
    }
    false
}

fn parent_key_for(container_key: &str, scope: Scope) -> Option<String> {
    match scope {
        Scope::Module => None,
        Scope::Class { .. } => Some(container_key.to_owned()),
    }
}

fn nested_parent(container_key: &str, outer: Scope) -> Option<String> {
    parent_key_for(container_key, outer)
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

/// A class or def block: decorators, a header, and an indented body.
fn container_doc(decorators: Vec<Doc>, header: Doc, members: Vec<Doc>) -> Doc {
    let mut parts = Vec::new();
    for decorator in decorators {
        parts.push(decorator);
        parts.push(Doc::Line);
    }
    parts.push(header);
    if members.is_empty() {
        parts.push(Doc::Text(" ...".to_owned()));
        return Doc::Concat(parts);
    }
    let mut body = Vec::new();
    for (index, member) in members.into_iter().enumerate() {
        if index > 0 {
            body.push(Doc::Line);
        }
        body.push(member);
    }
    parts.push(Doc::Line);
    parts.push(Doc::Indent(Box::new(Doc::Concat(body))));
    Doc::Concat(parts)
}

/// A function signature, optionally preceded by its preserved decorators.
fn signature_doc(decorators: Vec<Doc>, header: Doc) -> Doc {
    if decorators.is_empty() {
        return header;
    }
    let mut parts = Vec::new();
    for decorator in decorators {
        parts.push(decorator);
        parts.push(Doc::Line);
    }
    parts.push(header);
    Doc::Concat(parts)
}

fn span_of(node: Node<'_>) -> SourceSpan {
    syntax::node_span(node)
}
