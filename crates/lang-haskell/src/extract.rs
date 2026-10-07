//! Inspection of a parsed Haskell syntax tree into the shared projection model.
//!
//! Extraction follows the Haskell projection rules: the declared module header
//! is retained with its export list omitted, imports, comments, and Haddocks
//! are dropped, and only top-level and class/instance-level declarations are
//! considered. Type declarations, classes, and instances appear in both modes;
//! signatures, foreign imports, and pattern-synonym signatures are signatures.

use std::collections::HashSet;

use base::{
    ItemKind, KeyAllocator, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput,
    ProjectionMode, SourceSpan,
};
use tree_sitter::Node;

use crate::render::{self, Doc, Renderer};
use crate::syntax::{self, field, node};

struct Built {
    doc: Doc,
    items: Vec<ProjectedItem>,
}

struct Builder<'a> {
    renderer: &'a Renderer<'a>,
    mode: ProjectionMode,
    items: Vec<ProjectedItem>,
    keys: KeyAllocator,
}

pub(crate) fn project_file(input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
    let tree = syntax::parse(input.source, input.path)?;
    let root = tree.root_node();

    // Test detection is not implemented for Haskell: a test's name is a string
    // literal inside a body, which is never projected. Retaining nothing keeps
    // an empty projection meaning "no tests retained", as in Rust and Python.
    match input.mode {
        ProjectionMode::Types | ProjectionMode::Signatures => {}
        ProjectionMode::Tests => {
            return ProjectedFile::try_new(input.path.clone(), Vec::new());
        }
    }

    let renderer = Renderer::new(input.path, input.source);
    let mut builder = Builder {
        renderer: &renderer,
        mode: input.mode,
        items: Vec::new(),
        keys: KeyAllocator::new(),
    };
    builder.run(root)?;
    ProjectedFile::try_new(input.path.clone(), builder.items)
}

impl Builder<'_> {
    fn unique(&mut self, base: String) -> String {
        self.keys.unique(base)
    }

    fn run(&mut self, root: Node<'_>) -> Result<(), ProjectionError> {
        let mut pending: Vec<String> = Vec::new();
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            match child.kind() {
                node::PRAGMA => pending.push(self.renderer.node_text(child)),
                node::COMMENT | node::HADDOCK => {}
                "header" => {
                    let prefixes = std::mem::take(&mut pending);
                    if let Some(built) = self.module_decl(child, prefixes)? {
                        self.items.extend(built);
                    }
                }
                "declarations" => {
                    let prefixes = std::mem::take(&mut pending);
                    self.declarations(child, prefixes)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn module_decl(
        &mut self,
        header: Node<'_>,
        prefixes: Vec<String>,
    ) -> Result<Option<Vec<ProjectedItem>>, ProjectionError> {
        let Some(module) = self.renderer.field(header, field::MODULE) else {
            return Ok(None);
        };
        let name = self.renderer.node_text(module);
        let key = self.unique(format!("{}::module::{name}", self.renderer.path()));
        let text = format!("module {name}");
        let doc = with_prefix(prefixes, Doc::Text(text));
        let canonical_text = render::render(&doc, 0);
        Ok(Some(vec![ProjectedItem {
            stable_key: key,
            parent_key: None,
            kind: ItemKind::Module,
            name,
            span: syntax::node_span(header),
            canonical_text,
        }]))
    }

    fn declarations(
        &mut self,
        declarations: Node<'_>,
        prefixes: Vec<String>,
    ) -> Result<(), ProjectionError> {
        let nodes: Vec<Node<'_>> = {
            let mut cursor = declarations.walk();
            declarations
                .named_children(&mut cursor)
                .filter(|child| !matches!(child.kind(), node::COMMENT | node::HADDOCK))
                .collect()
        };

        // Definitions paired with an explicit signature are represented by the
        // signature alone; collect signature names up front so a definition can
        // be skipped regardless of source order.
        let mut signed: HashSet<String> = HashSet::new();
        for node in &nodes {
            if node.kind() == node::SIGNATURE {
                signed.extend(self.renderer.signature_names(*node));
            }
        }

        let mut pending = prefixes;
        for node in nodes {
            let (items, consumed) = self.declaration(node, &signed, pending)?;
            self.items.extend(items);
            pending = consumed;
        }
        Ok(())
    }

    fn declaration(
        &mut self,
        node: Node<'_>,
        signed: &HashSet<String>,
        prefixes: Vec<String>,
    ) -> Result<(Vec<ProjectedItem>, Vec<String>), ProjectionError> {
        if node.kind() == node::PRAGMA {
            let mut pending = prefixes;
            pending.push(self.renderer.node_text(node));
            return Ok((Vec::new(), pending));
        }

        let built = match node.kind() {
            node::DATA_TYPE => Some(self.data_decl(node, prefixes, "data")),
            node::NEWTYPE => Some(self.data_decl(node, prefixes, "newtype")),
            node::TYPE_SYNONYM => Some(self.type_synonym_decl(node, prefixes)),
            node::KIND_SIGNATURE => Some(self.simple_decl(node, prefixes, ItemKind::Type, "kind")),
            node::TYPE_ROLE => Some(self.simple_decl(node, prefixes, ItemKind::Type, "role")),
            node::TYPE_FAMILY | node::DATA_FAMILY => {
                let keyword = if node.kind() == node::TYPE_FAMILY {
                    "type family"
                } else {
                    "data family"
                };
                Some(self.family_decl(node, prefixes, keyword))
            }
            node::TYPE_INSTANCE | node::DATA_INSTANCE => {
                Some(self.simple_decl(node, prefixes, ItemKind::AssociatedType, "instance"))
            }
            node::DERIVING_INSTANCE => {
                Some(self.simple_decl(node, prefixes, ItemKind::TraitImplementation, "deriving"))
            }
            node::CLASS => Some(self.class_decl(node, prefixes, "class", ItemKind::Trait)?),
            "instance" => {
                Some(self.class_decl(node, prefixes, "instance", ItemKind::TraitImplementation)?)
            }
            node::SIGNATURE => {
                if self.mode == ProjectionMode::Signatures {
                    Some(self.value_decl(
                        node,
                        prefixes,
                        ItemKind::Function,
                        self.renderer.signature_doc(node),
                    ))
                } else {
                    None
                }
            }
            node::FUNCTION => {
                if self.mode == ProjectionMode::Signatures {
                    let name = self
                        .renderer
                        .field_text(node, field::NAME)
                        .unwrap_or_default();
                    let head = self.renderer.function_head(node);
                    if head.is_empty() || signed.contains(&name) {
                        None
                    } else {
                        Some(self.value_decl(node, prefixes, ItemKind::Function, Doc::Text(head)))
                    }
                } else {
                    None
                }
            }
            node::BIND => {
                if self.mode == ProjectionMode::Signatures {
                    match self.renderer.bind_name(node) {
                        Some(name) if !signed.contains(&name) => {
                            Some(self.value_decl(node, prefixes, ItemKind::Value, Doc::Text(name)))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            node::FOREIGN_IMPORT => {
                if self.mode == ProjectionMode::Signatures {
                    Some(self.simple_decl(node, prefixes, ItemKind::Function, "foreign"))
                } else {
                    None
                }
            }
            node::PATTERN_SYNONYM => {
                if self.mode == ProjectionMode::Signatures
                    && self.renderer.field(node, "signature").is_some()
                {
                    Some(self.simple_decl(node, prefixes, ItemKind::PatternSynonym, "pattern"))
                } else {
                    None
                }
            }
            _ => None,
        };

        match built {
            Some(built) => Ok((built.items, Vec::new())),
            None => Ok((Vec::new(), Vec::new())),
        }
    }

    fn name_of(&self, node: Node<'_>) -> String {
        self.renderer
            .field_text(node, field::NAME)
            .or_else(|| self.renderer.field_text(node, "synonym"))
            .or_else(|| self.renderer.field_text(node, "type"))
            .unwrap_or_default()
    }

    fn simple_decl(
        &mut self,
        node: Node<'_>,
        prefixes: Vec<String>,
        kind: ItemKind,
        tag: &str,
    ) -> Built {
        let name = self.name_of(node);
        let key = self.unique(format!("{}::{}::{name}", self.renderer.path(), tag));
        let text = self.renderer.node_text(node);
        self.make(
            node,
            key,
            None,
            kind,
            name,
            with_prefix(prefixes, Doc::Text(text)),
            0,
            Vec::new(),
        )
    }

    // A `type` synonym, whose right-hand side wraps through the renderer's
    // recursive type document.
    fn type_synonym_decl(&mut self, node: Node<'_>, prefixes: Vec<String>) -> Built {
        let name = self.name_of(node);
        let key = self.unique(format!("{}::alias::{name}", self.renderer.path()));
        let doc = with_prefix(prefixes, self.renderer.type_synonym_doc(node));
        self.make(
            node,
            key,
            None,
            ItemKind::TypeAlias,
            name,
            doc,
            0,
            Vec::new(),
        )
    }

    fn value_decl(
        &mut self,
        node: Node<'_>,
        prefixes: Vec<String>,
        kind: ItemKind,
        doc: Doc,
    ) -> Built {
        let name = self.name_of(node);
        let key = self.unique(format!(
            "{}::{}::{name}",
            self.renderer.path(),
            kind_tag(kind)
        ));
        self.make(
            node,
            key,
            None,
            kind,
            name,
            with_prefix(prefixes, doc),
            0,
            Vec::new(),
        )
    }

    fn data_decl(&mut self, node: Node<'_>, prefixes: Vec<String>, keyword: &str) -> Built {
        let name = self.name_of(node);
        let key = self.unique(format!("{}::type::{name}", self.renderer.path()));
        let doc = with_prefix(prefixes, self.renderer.data_doc(node, keyword));
        self.make(node, key, None, ItemKind::Type, name, doc, 0, Vec::new())
    }

    fn family_decl(&mut self, node: Node<'_>, prefixes: Vec<String>, keyword: &str) -> Built {
        let name = self.name_of(node);
        let key = self.unique(format!("{}::family::{name}", self.renderer.path()));
        let doc = with_prefix(prefixes, self.renderer.family_doc(node, keyword));
        self.make(
            node,
            key,
            None,
            ItemKind::TypeFamily,
            name,
            doc,
            0,
            Vec::new(),
        )
    }

    fn class_decl(
        &mut self,
        node: Node<'_>,
        prefixes: Vec<String>,
        keyword: &str,
        kind: ItemKind,
    ) -> Result<Built, ProjectionError> {
        let name = self.name_of(node);
        let key = self.unique(format!("{}::{keyword}::{name}", self.renderer.path()));
        let members = self.members(node, &key)?;
        let header = self.renderer.class_header(node, keyword);
        let doc = with_prefix(prefixes, container_doc(header, member_docs(&members)));
        let nested = member_items(members);
        Ok(self.make(node, key, None, kind, name, doc, 0, nested))
    }

    fn members(
        &mut self,
        node: Node<'_>,
        container_key: &str,
    ) -> Result<Vec<Built>, ProjectionError> {
        let Some(declarations) = self.renderer.field(node, "declarations") else {
            return Ok(Vec::new());
        };
        // A class or instance method that also has an explicit signature is
        // represented by the signature alone; its definition only contributes a
        // head when no signature exists.
        let mut signed: HashSet<String> = HashSet::new();
        {
            let mut cursor = declarations.walk();
            for child in declarations.named_children(&mut cursor) {
                if matches!(child.kind(), node::SIGNATURE | "default_signature") {
                    signed.extend(self.renderer.signature_names(child));
                }
            }
        }

        let mut members = Vec::new();
        let mut cursor = declarations.walk();
        for child in declarations.named_children(&mut cursor) {
            let built = match child.kind() {
                node::COMMENT | node::HADDOCK => None,
                node::SIGNATURE | "default_signature" => {
                    if self.mode == ProjectionMode::Signatures {
                        Some(self.member(child, container_key, self.renderer.signature_doc(child)))
                    } else {
                        None
                    }
                }
                node::FUNCTION => {
                    if self.mode == ProjectionMode::Signatures {
                        let name = self
                            .renderer
                            .field_text(child, field::NAME)
                            .unwrap_or_default();
                        let head = self.renderer.function_head(child);
                        if head.is_empty() || signed.contains(&name) {
                            None
                        } else {
                            Some(self.member(child, container_key, Doc::Text(head)))
                        }
                    } else {
                        None
                    }
                }
                node::BIND => {
                    if self.mode == ProjectionMode::Signatures {
                        match self.renderer.bind_name(child) {
                            Some(name) if !signed.contains(&name) => {
                                Some(self.member(child, container_key, Doc::Text(name)))
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                node::TYPE_INSTANCE | node::DATA_INSTANCE => Some(self.member(
                    child,
                    container_key,
                    Doc::Text(self.renderer.node_text(child)),
                )),
                _ => None,
            };
            if let Some(built) = built {
                members.push(built);
            }
        }
        Ok(members)
    }

    fn member(&mut self, node: Node<'_>, container_key: &str, doc: Doc) -> Built {
        let name = self.name_of(node);
        let key = self.unique(format!("{container_key}::method::{name}"));
        self.make(
            node,
            key,
            Some(container_key.to_owned()),
            ItemKind::Method,
            name,
            doc,
            1,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn make(
        &self,
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
}

fn kind_tag(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Module => "module",
        ItemKind::Type => "type",
        ItemKind::TypeAlias => "alias",
        ItemKind::Constructor => "constructor",
        ItemKind::Field => "field",
        ItemKind::Variant => "variant",
        ItemKind::Trait => "trait",
        ItemKind::TraitImplementation => "impl",
        ItemKind::AssociatedType => "assoc",
        ItemKind::TypeFamily => "family",
        ItemKind::PatternSynonym => "pattern",
        ItemKind::Function => "fn",
        ItemKind::Method => "method",
        ItemKind::Value => "value",
        ItemKind::Constant => "const",
        ItemKind::Static => "static",
        ItemKind::Port => "port",
        ItemKind::Operator => "operator",
        ItemKind::ForeignBlock => "foreign",
    }
}

fn with_prefix(prefixes: Vec<String>, doc: Doc) -> Doc {
    if prefixes.is_empty() {
        return doc;
    }
    let mut parts = Vec::new();
    for prefix in prefixes {
        parts.push(Doc::Text(prefix));
        parts.push(Doc::Line);
    }
    parts.push(doc);
    Doc::Concat(parts)
}

// A class or instance block: a header and an indented member list. The `where`
// keyword is only emitted when a member is actually shown.
fn container_doc(header: String, members: Vec<Doc>) -> Doc {
    if members.is_empty() {
        return Doc::Text(header);
    }
    let mut body = Vec::new();
    for (index, member) in members.into_iter().enumerate() {
        if index > 0 {
            body.push(Doc::Line);
        }
        body.push(member);
    }
    Doc::Concat(vec![
        Doc::Text(format!("{header} where")),
        Doc::Line,
        Doc::Indent(Box::new(Doc::Concat(body))),
    ])
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

fn span_of(node: Node<'_>) -> SourceSpan {
    syntax::node_span(node)
}
