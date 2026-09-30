//! Canonical rendering of Haskell declarations.
//!
//! Rendering is independent of original whitespace and comments so that
//! formatting-only edits do not appear in focused diffs. Declarations are
//! rebuilt from leaf tokens with fixed spacing rules and fixed structural line
//! breaks; source slices are used only for atomic literals. Terminal width is
//! never consulted.

pub(crate) use base::doc::{Doc, render};
use base::{RepoPath, SupportedPath};
use tree_sitter::Node;

use crate::syntax;

pub(crate) struct Renderer<'a> {
    path: &'a SupportedPath,
    source: &'a str,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(path: &'a SupportedPath, source: &'a str) -> Self {
        Self { path, source }
    }

    pub(crate) fn path(&self) -> &RepoPath {
        self.path.path()
    }

    pub(crate) fn slice(&self, node: Node<'_>) -> &str {
        self.source.get(node.byte_range()).unwrap_or("")
    }

    pub(crate) fn field<'n>(&self, node: Node<'n>, name: &str) -> Option<Node<'n>> {
        node.child_by_field_name(name)
    }

    pub(crate) fn field_text(&self, node: Node<'_>, name: &str) -> Option<String> {
        self.field(node, name).map(|child| self.node_text(child))
    }

    fn child_of_kind<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| child.kind() == kind)
    }

    fn push_tokens(&self, node: Node<'_>, out: &mut Vec<String>) {
        let kind = node.kind();
        if kind == syntax::COMMENT || kind == syntax::HADDOCK {
            return;
        }
        // Strings and characters carry meaningful internal bytes.
        if matches!(kind, "string" | "char") {
            out.push(self.slice(node).to_owned());
            return;
        }
        if matches!(kind, "integer" | "float") {
            out.push(self.slice(node).to_owned());
            return;
        }
        if node.child_count() == 0 {
            out.push(self.slice(node).to_owned());
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.push_tokens(child, out);
        }
    }

    /// Canonical text for a type, signature, or declaration fragment, with
    /// comments removed and spacing normalized.
    pub(crate) fn node_text(&self, node: Node<'_>) -> String {
        let mut tokens = Vec::new();
        self.push_tokens(node, &mut tokens);
        join(&tokens)
    }

    /// A `type_params` fragment, prefixed with a single leading space.
    pub(crate) fn params_text(&self, node: Option<Node<'_>>) -> String {
        match node {
            Some(node) => {
                let text = self.node_text(node);
                if text.is_empty() {
                    String::new()
                } else {
                    format!(" {text}")
                }
            }
            None => String::new(),
        }
    }

    /// A `context` fragment (`Eq a =>`) with a trailing space.
    pub(crate) fn context_text(&self, node: Option<Node<'_>>) -> String {
        match node {
            Some(node) => {
                let text = self.node_text(node);
                if text.is_empty() {
                    String::new()
                } else {
                    format!("{text} ")
                }
            }
            None => String::new(),
        }
    }

    /// A list of `field` fragments, each `name :: type`.
    pub(crate) fn record_fields(&self, fields: Node<'_>) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = fields.walk();
        for child in fields.named_children(&mut cursor) {
            if child.kind() == syntax::FIELD {
                out.push(self.node_text(child));
            }
        }
        out
    }

    /// A data or newtype declaration, with record fields and constructors one
    /// per indented line.
    pub(crate) fn data_doc(&self, node: Node<'_>, keyword: &str) -> Doc {
        let name = self
            .field_text(node, syntax::FIELD_NAME_FIELD)
            .unwrap_or_default();
        let context = self.context_text(self.field(node, "context"));
        let params = self.params_text(self.field(node, "patterns"));
        let kind = self
            .field(node, "kind")
            .map(|kind| format!(" :: {}", self.node_text(kind)))
            .unwrap_or_default();
        let header = format!("{keyword} {context}{name}{params}{kind}");

        let mut lines: Vec<Doc> = Vec::new();

        // GADT syntax: `data T where` with one `Name :: Type` per line.
        if let Some(constructors) = self.field(node, "constructors")
            && constructors.kind() == "gadt_constructors"
        {
            lines.push(Doc::Text(format!("{header} where")));
            let mut body = Vec::new();
            let mut cursor = constructors.walk();
            for child in constructors.named_children(&mut cursor) {
                if child.kind() == "gadt_constructor" {
                    if !body.is_empty() {
                        body.push(Doc::Line);
                    }
                    body.push(Doc::Text(self.node_text(child)));
                }
            }
            lines.push(Doc::Indent(Box::new(Doc::Concat(body))));
        } else if let Some(constructors) = self.field(node, "constructors") {
            let members = self.constructor_texts(constructors);
            // A single record constructor reads as a record definition.
            if let Some(record) = self.single_record(constructors) {
                let ctor_name = self
                    .field_text(record, "name")
                    .unwrap_or_else(|| members.first().cloned().unwrap_or_default());
                lines.push(Doc::Text(format!("{header} = {ctor_name}")));
                lines.push(self.record_block(record));
            } else {
                lines.push(Doc::Text(header));
                let mut body = Vec::new();
                for (index, member) in members.into_iter().enumerate() {
                    if index > 0 {
                        body.push(Doc::Line);
                    }
                    let prefix = if index == 0 { "= " } else { "| " };
                    body.push(Doc::Text(format!("{prefix}{member}")));
                }
                lines.push(Doc::Indent(Box::new(Doc::Concat(body))));
            }
        } else if let Some(constructor) = self.field(node, "constructor") {
            // `newtype Email = Email Text`, or a record newtype.
            match self.child_of_kind(constructor, "record") {
                Some(record) => {
                    let ctor_name = self
                        .field_text(constructor, "name")
                        .unwrap_or_else(|| self.node_text(constructor));
                    lines.push(Doc::Text(format!("{header} = {ctor_name}")));
                    lines.push(self.record_block(record));
                }
                None => lines.push(Doc::Text(format!(
                    "{header} = {}",
                    self.node_text(constructor)
                ))),
            }
        } else {
            lines.push(Doc::Text(header));
        }

        // `deriving` clauses are one per line, indented with the body.
        let mut cursor = node.walk();
        let deriving: Vec<Node<'_>> = node
            .children_by_field_name("deriving", &mut cursor)
            .collect();
        if !deriving.is_empty() {
            let mut body = Vec::new();
            for clause in deriving {
                if !body.is_empty() {
                    body.push(Doc::Line);
                }
                body.push(Doc::Text(self.node_text(clause)));
            }
            lines.push(Doc::Indent(Box::new(Doc::Concat(body))));
        }

        join_lines(lines)
    }

    fn constructor_texts(&self, constructors: Node<'_>) -> Vec<String> {
        let mut out = Vec::new();
        let mut cursor = constructors.walk();
        for child in constructors.named_children(&mut cursor) {
            if child.kind() == syntax::DATA_CONSTRUCTOR {
                out.push(self.node_text(child));
            }
        }
        out
    }

    fn single_record<'t>(&self, constructors: Node<'t>) -> Option<Node<'t>> {
        let named: Vec<Node<'t>> = {
            let mut cursor = constructors.walk();
            constructors.named_children(&mut cursor).collect()
        };
        if named.len() != 1 {
            return None;
        }
        let constructor = named[0];
        let inner = self.field(constructor, "constructor")?;
        if inner.kind() == "record" {
            Some(inner)
        } else {
            None
        }
    }

    /// A record body as `{ field, field }`, one field per line.
    pub(crate) fn record_block(&self, record: Node<'_>) -> Doc {
        let fields = self.record_field_texts(record);
        if fields.is_empty() {
            return Doc::Text("{}".to_owned());
        }
        let mut body = Vec::new();
        for (index, field) in fields.into_iter().enumerate() {
            if index > 0 {
                body.push(Doc::Line);
            }
            let prefix = if index == 0 { "{ " } else { ", " };
            body.push(Doc::Text(format!("{prefix}{field}")));
        }
        body.push(Doc::Line);
        body.push(Doc::Text("}".to_owned()));
        Doc::Indent(Box::new(Doc::Concat(body)))
    }

    /// The `field` fragments of a record, whether they sit under a `fields`
    /// wrapper (`data`) or directly on the record (`newtype`).
    fn record_field_texts(&self, record: Node<'_>) -> Vec<String> {
        if let Some(fields) = self.field(record, "fields") {
            return self.record_fields(fields);
        }
        let mut out = Vec::new();
        let mut cursor = record.walk();
        for child in record.children_by_field_name("field", &mut cursor) {
            if child.kind() == syntax::FIELD {
                out.push(self.node_text(child));
            }
        }
        out
    }

    /// A type or data family, with a closed family's equations one per indented
    /// line.
    pub(crate) fn family_doc(&self, node: Node<'_>, keyword: &str) -> Doc {
        let name = self
            .field_text(node, syntax::FIELD_NAME_FIELD)
            .unwrap_or_default();
        let params = self.params_text(self.field(node, "patterns"));
        let header = format!("{keyword} {name}{params}");

        let Some(closed) = self.field(node, "closed_family") else {
            return Doc::Text(header);
        };
        if closed.kind() != "equations" {
            return Doc::Text(header);
        }
        let mut body = Vec::new();
        let mut cursor = closed.walk();
        for child in closed.named_children(&mut cursor) {
            if !body.is_empty() {
                body.push(Doc::Line);
            }
            body.push(Doc::Text(self.node_text(child)));
        }
        if body.is_empty() {
            return Doc::Text(header);
        }
        Doc::Concat(vec![
            Doc::Text(format!("{header} where")),
            Doc::Line,
            Doc::Indent(Box::new(Doc::Concat(body))),
        ])
    }

    /// A class or instance header without the `where` keyword.
    pub(crate) fn class_header(&self, node: Node<'_>, keyword: &str) -> String {
        let context = self.context_text(self.field(node, "context"));
        let forall = self
            .field(node, "forall")
            .map(|forall| format!("{} ", self.node_text(forall)))
            .unwrap_or_default();
        let name = self
            .field_text(node, syntax::FIELD_NAME_FIELD)
            .unwrap_or_default();
        let params = self.params_text(self.field(node, "patterns"));
        let fundeps = self
            .field(node, "fundeps")
            .map(|fundeps| format!(" | {}", self.node_text(fundeps)))
            .unwrap_or_default();
        format!("{keyword} {context}{forall}{name}{params}{fundeps}")
    }

    /// A signature declaration, e.g. `f, g :: Int -> Int`, wrapping the type's
    /// top-level arrows when it does not fit the line budget.
    pub(crate) fn signature_doc(&self, node: Node<'_>) -> Doc {
        let names = match self.field(node, "names") {
            Some(names) => self.node_text(names),
            None => self
                .field_text(node, syntax::FIELD_NAME_FIELD)
                .unwrap_or_default(),
        };
        let Some(type_node) = self.field(node, syntax::FIELD_TYPE) else {
            return Doc::Text(format!("{names} ::"));
        };
        if matches!(type_node.kind(), syntax::FUNCTION | syntax::CONTEXT) {
            // Arrow chains (and a leading context) already break at their own
            // `->`/`=>` structure, so keep the type on the `::` line.
            return Doc::Concat(vec![
                Doc::Text(format!("{names} :: ")),
                self.type_doc(type_node),
            ]);
        }
        // A single non-arrow atom cannot break internally in the common case,
        // so allow it to move to an indented line after `::`.
        Doc::Concat(vec![
            Doc::Text(format!("{names} ::")),
            Doc::Group(Box::new(Doc::Indent(Box::new(Doc::Concat(vec![
                Doc::SoftLine,
                self.type_doc(type_node),
            ]))))),
        ])
    }

    /// A recursive type document. Long bracketed atoms (`parens`, `tuple`,
    /// `list`, `apply`, `infix`) and arrow chains break at their own structure
    /// when they do not fit; every other node kind stays flat.
    pub(crate) fn type_doc(&self, node: Node<'_>) -> Doc {
        match node.kind() {
            syntax::FUNCTION | syntax::CONTEXT => self.arrow_type_doc(node),
            "parens" => self.parens_type_doc(node),
            "tuple" => self.bracket_elements_doc(node, "element", "(", ")", true),
            "unboxed_tuple" => self.bracket_elements_doc(node, "element", "(# ", " #)", false),
            "list" => self.list_type_doc(node),
            "apply" => self.apply_type_doc(node),
            "infix" => self.infix_type_doc(node),
            _ => Doc::Text(self.node_text(node)),
        }
    }

    /// A type synonym: `type Name params =` followed by an indented, wrapped
    /// right-hand side.
    pub(crate) fn type_synonym_doc(&self, node: Node<'_>) -> Doc {
        let name = self
            .field_text(node, syntax::FIELD_NAME_FIELD)
            .unwrap_or_default();
        let params = self.params_text(self.field(node, "patterns"));
        let header = format!("type {name}{params} =");
        let Some(rhs) = self.field(node, syntax::FIELD_TYPE) else {
            return Doc::Text(header);
        };
        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text(header),
            Doc::Indent(Box::new(Doc::Concat(vec![
                Doc::SoftLine,
                Doc::Group(Box::new(self.type_doc(rhs))),
            ]))),
        ])))
    }

    /// A function type's top-level arrow chain (and a leading `C a =>` context),
    /// rendered with each parameter and result as a recursive [`type_doc`].
    fn arrow_type_doc(&self, node: Node<'_>) -> Doc {
        let mut node = node;
        let mut head = None;
        if node.kind() == syntax::CONTEXT {
            let context = self
                .field(node, "context")
                .map(|context| self.node_text(context))
                .unwrap_or_default();
            head = Some(Doc::Text(context));
            match self.field(node, syntax::FIELD_TYPE) {
                Some(inner) => node = inner,
                None => return head.unwrap_or_else(|| Doc::Text(String::new())),
            }
        }

        let mut atoms: Vec<Doc> = Vec::new();
        let mut current = node;
        loop {
            if current.kind() == syntax::FUNCTION {
                if let Some(parameter) = self.field(current, "parameter") {
                    atoms.push(self.type_doc(parameter));
                }
                match self.field(current, "result") {
                    Some(result) => current = result,
                    None => break,
                }
            } else {
                atoms.push(self.type_doc(current));
                break;
            }
        }

        let mut atoms = atoms.into_iter();
        let mut rest: Vec<Doc> = Vec::new();
        let head = match head {
            Some(head) => {
                rest.push(Doc::SoftLine);
                rest.push(Doc::Text("=> ".to_owned()));
                if let Some(first) = atoms.next() {
                    rest.push(first);
                }
                head
            }
            None => atoms.next().unwrap_or_else(|| Doc::Text(String::new())),
        };
        for atom in atoms {
            rest.push(Doc::SoftLine);
            rest.push(Doc::Text("-> ".to_owned()));
            rest.push(atom);
        }

        Doc::Group(Box::new(Doc::Concat(vec![
            head,
            Doc::Indent(Box::new(Doc::Concat(rest))),
        ])))
    }

    fn parens_type_doc(&self, node: Node<'_>) -> Doc {
        let Some(inner) = self.field(node, syntax::FIELD_TYPE) else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text("(".to_owned()),
            Doc::Indent(Box::new(Doc::Concat(vec![
                Doc::SoftNil,
                self.type_doc(inner),
            ]))),
            Doc::SoftNil,
            Doc::Text(")".to_owned()),
        ])))
    }

    /// A bracketed list of `field` elements in the leading-comma block shape:
    /// `(A, B)` flat, `( A\n, B\n)` broken. `spaced_open` adds a space after the
    /// open delimiter only when broken.
    fn bracket_elements_doc(
        &self,
        node: Node<'_>,
        field: &str,
        open: &str,
        close: &str,
        spaced_open: bool,
    ) -> Doc {
        let mut cursor = node.walk();
        let elements: Vec<Node<'_>> = node.children_by_field_name(field, &mut cursor).collect();
        if elements.is_empty() {
            return Doc::Text(self.node_text(node));
        }

        let mut inner: Vec<Doc> = Vec::new();
        if spaced_open {
            inner.push(Doc::Broken(" "));
        }
        inner.push(self.type_doc(elements[0]));
        for element in &elements[1..] {
            inner.push(Doc::SoftNil);
            inner.push(Doc::Text(", ".to_owned()));
            inner.push(self.type_doc(*element));
        }
        inner.push(Doc::SoftNil);
        inner.push(Doc::Text(close.to_owned()));

        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text(open.to_owned()),
            Doc::Indent(Box::new(Doc::Concat(inner))),
        ])))
    }

    fn list_type_doc(&self, node: Node<'_>) -> Doc {
        let Some(element) = self.field(node, "element") else {
            return Doc::Text(self.node_text(node));
        };
        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text("[".to_owned()),
            Doc::Indent(Box::new(Doc::Concat(vec![
                Doc::SoftNil,
                self.type_doc(element),
            ]))),
            Doc::SoftNil,
            Doc::Text("]".to_owned()),
        ])))
    }

    fn apply_type_doc(&self, node: Node<'_>) -> Doc {
        match (
            self.field(node, "constructor"),
            self.field(node, "argument"),
        ) {
            (Some(constructor), Some(argument)) => Doc::Concat(vec![
                self.type_doc(constructor),
                Doc::Text(" ".to_owned()),
                self.type_doc(argument),
            ]),
            _ => Doc::Text(self.node_text(node)),
        }
    }

    /// An infix type chain, breaking before each operator like the arrow layout.
    fn infix_type_doc(&self, node: Node<'_>) -> Doc {
        let mut operands: Vec<Doc> = Vec::new();
        let mut operators: Vec<String> = Vec::new();
        let mut current = node;
        loop {
            if current.kind() == "infix" {
                if let Some(left) = self.field(current, "left_operand") {
                    operands.push(self.type_doc(left));
                }
                if let Some(operator) = self.field(current, "operator") {
                    operators.push(self.node_text(operator));
                }
                match self.field(current, "right_operand") {
                    Some(right) => current = right,
                    None => break,
                }
            } else {
                operands.push(self.type_doc(current));
                break;
            }
        }

        let mut operands = operands.into_iter();
        let first = operands.next().unwrap_or_else(|| Doc::Text(String::new()));
        let mut rest: Vec<Doc> = Vec::new();
        for (operand, operator) in operands.zip(operators) {
            rest.push(Doc::SoftLine);
            rest.push(Doc::Text(format!("{operator} ")));
            rest.push(operand);
        }

        Doc::Group(Box::new(Doc::Concat(vec![
            first,
            Doc::Indent(Box::new(Doc::Concat(rest))),
        ])))
    }

    /// The written head of a function definition, without the body.
    pub(crate) fn function_head(&self, node: Node<'_>) -> String {
        let name = self
            .field_text(node, syntax::FIELD_NAME_FIELD)
            .unwrap_or_default();
        let params = self
            .field(node, "patterns")
            .map(|patterns| format!(" {}", self.node_text(patterns)))
            .unwrap_or_default();
        format!("{name}{params}")
    }

    /// The written name of a binding, or `None` for a pattern binding.
    pub(crate) fn bind_name(&self, node: Node<'_>) -> Option<String> {
        self.field_text(node, syntax::FIELD_NAME_FIELD)
    }

    /// The names declared by a signature, for pairing with definitions.
    pub(crate) fn signature_names(&self, node: Node<'_>) -> Vec<String> {
        if let Some(names) = self.field(node, "names") {
            let mut out = Vec::new();
            let mut cursor = names.walk();
            for child in names.named_children(&mut cursor) {
                out.push(self.node_text(child));
            }
            out
        } else if let Some(name) = self.field_text(node, syntax::FIELD_NAME_FIELD) {
            vec![name]
        } else {
            Vec::new()
        }
    }
}

fn join_lines(lines: Vec<Doc>) -> Doc {
    let mut parts = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            parts.push(Doc::Line);
        }
        parts.push(line);
    }
    Doc::Concat(parts)
}

/// Spacing is a function of the two adjacent leaf tokens, never of source
/// whitespace.
pub(crate) fn join(tokens: &[String]) -> String {
    let mut text = String::new();
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 {
            let previous = &tokens[index - 1];
            let before_dot = index.checked_sub(2).map(|i| tokens[i].as_str());
            if needs_space(previous, token, before_dot) {
                text.push(' ');
            }
        }
        text.push_str(token);
    }
    text
}

fn needs_space(previous: &str, next: &str, before_dot: Option<&str>) -> bool {
    if previous.is_empty() || next.is_empty() {
        return false;
    }

    // Punctuation that attaches to the preceding token.
    if matches!(next, ")" | "]" | "}" | "," | ";") {
        return false;
    }

    // Opening delimiters and prefix punctuation attach to what follows.
    if matches!(previous, "(" | "[" | "{" | "'" | "!" | "~" | "`") {
        return false;
    }

    // Haskell type application separates a constructor from its argument, so a
    // bracket after a name (`deriving (Eq, Show)`, `Maybe (Either a b)`) keeps a
    // space.
    if matches!(next, "(" | "[" | "{") {
        return true;
    }

    // A dot binds a qualified name (`Data.Text`) but separates a `forall`
    // binder from its body (`forall a. a`). A dot is part of a qualified name
    // when the segment before it, or the segment after it, starts uppercase.
    if next == "." {
        return false;
    }
    if previous == "." {
        let qualified = before_dot
            .and_then(|before| before.chars().next())
            .is_some_and(char::is_uppercase)
            || next.chars().next().is_some_and(char::is_uppercase);
        return !qualified;
    }

    // Separators and infix operators are surrounded by one space.
    if matches!(
        previous,
        "," | ";" | "::" | "->" | "=>" | "=" | "|" | "<-" | "~"
    ) {
        return true;
    }
    if matches!(
        previous,
        "+" | "-" | "*" | "/" | "^" | "++" | ":" | "==" | "/=" | "<" | ">" | "<=" | ">="
    ) {
        return true;
    }
    if matches!(
        next,
        "::" | "->" | "=>" | "=" | "|" | "<-" | "+" | "-" | "*" | "/" | "^" | "++" | ":"
    ) {
        return true;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    fn spaced(parts: &[&str]) -> String {
        let tokens: Vec<String> = parts.iter().map(|part| (*part).to_owned()).collect();
        join(&tokens)
    }

    /// Renders the first `type` synonym in `source` at the top level.
    fn render_type_synonym(source: &str) -> String {
        let path = supported("Temp.hs");
        let tree = syntax::parse(source, &path).expect("the source parses");
        let renderer = Renderer::new(&path, source);
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == syntax::TYPE_SYNONYM {
                return render(&renderer.type_synonym_doc(node), 0);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        panic!("no type synonym in {source:?}");
    }

    /// Renders the first `signature` in `source` at the top level.
    fn render_signature(source: &str) -> String {
        let path = supported("Temp.hs");
        let tree = syntax::parse(source, &path).expect("the source parses");
        let renderer = Renderer::new(&path, source);
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == syntax::SIGNATURE {
                return render(&renderer.signature_doc(node), 0);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        panic!("no signature in {source:?}");
    }

    #[test]
    fn short_single_atom_signature_stays_inline() {
        assert_eq!(render_signature("f :: Maybe a"), "f :: Maybe a");
    }

    #[test]
    fn long_parenthesized_signature_breaks_after_the_double_colon() {
        let source = "journalWithHistoricalCostsUsing :: (Day -> Hledger.MixedAmount -> Hledger.MixedAmount)";
        assert_eq!(
            render_signature(source),
            concat!(
                "journalWithHistoricalCostsUsing ::\n",
                "    (Day -> Hledger.MixedAmount -> Hledger.MixedAmount)",
            )
        );
    }

    #[test]
    fn arrow_signature_breaks_before_the_arrows() {
        let source = "combine :: String -> Int -> [String] -> OrganizationId -> AccountSettings -> BillingProfile -> IO (Either Error User)";
        assert_eq!(
            render_signature(source),
            concat!(
                "combine :: String\n",
                "    -> Int\n",
                "    -> [String]\n",
                "    -> OrganizationId\n",
                "    -> AccountSettings\n",
                "    -> BillingProfile\n",
                "    -> IO (Either Error User)",
            )
        );
    }

    #[test]
    fn short_type_synonym_stays_inline() {
        assert_eq!(
            render_type_synonym("type UserId = Int"),
            "type UserId = Int"
        );
    }

    #[test]
    fn short_type_atoms_stay_inline() {
        assert_eq!(
            render_type_synonym("type Pair a = (a, a)"),
            "type Pair a = (a, a)"
        );
        assert_eq!(
            render_type_synonym("type Wrap a = Gen (a, a)"),
            "type Wrap a = Gen (a, a)"
        );
        assert_eq!(
            render_type_synonym("type F = (Int -> Bool)"),
            "type F = (Int -> Bool)"
        );
    }

    #[test]
    fn long_type_synonym_breaks_before_the_operators() {
        let source = "type API = \"api\" :> \"v1\" :> Header \"X-Request-ID\" Text :> (SystemAPI :<|> AccountsAPI :<|> TransactionsAPI)";
        assert_eq!(
            render_type_synonym(source),
            concat!(
                "type API =\n",
                "    \"api\"\n",
                "        :> \"v1\"\n",
                "        :> Header \"X-Request-ID\" Text\n",
                "        :> (SystemAPI :<|> AccountsAPI :<|> TransactionsAPI)",
            )
        );
    }

    #[test]
    fn long_tuple_type_uses_the_leading_comma_block() {
        let source = "type Big = (Text.Text, AssetClassMappings, InvestmentMappings, Maybe Text.Text, Extra, More)";
        assert_eq!(
            render_type_synonym(source),
            concat!(
                "type Big =\n",
                "    ( Text.Text\n",
                "        , AssetClassMappings\n",
                "        , InvestmentMappings\n",
                "        , Maybe Text.Text\n",
                "        , Extra\n",
                "        , More\n",
                "        )",
            )
        );
    }

    #[test]
    fn long_application_breaks_its_tuple_argument() {
        let source = "type GenResponse = Gen (Text.Text, AssetClassMappings, InvestmentMappings, Maybe Text.Text, Extra, More)";
        assert_eq!(
            render_type_synonym(source),
            concat!(
                "type GenResponse =\n",
                "    Gen ( Text.Text\n",
                "        , AssetClassMappings\n",
                "        , InvestmentMappings\n",
                "        , Maybe Text.Text\n",
                "        , Extra\n",
                "        , More\n",
                "        )",
            )
        );
    }

    #[test]
    fn spacing_normalizes_haskell_fragments() {
        assert_eq!(spaced(&["f", "::", "Int", "->", "Int"]), "f :: Int -> Int");
        assert_eq!(
            spaced(&["Eq", "a", "=>", "a", "->", "Bool"]),
            "Eq a => a -> Bool"
        );
        assert_eq!(spaced(&["Maybe", "a"]), "Maybe a");
        assert_eq!(spaced(&["[", "a", "]"]), "[a]");
        assert_eq!(spaced(&["(", ",", ",", ")"]), "(,,)");
        assert_eq!(
            spaced(&["Data", ".", "Text", ".", "pack"]),
            "Data.Text.pack"
        );
        assert_eq!(
            spaced(&["forall", "a", ".", "a", "->", "a"]),
            "forall a. a -> a"
        );
    }
}
