//! The shared projection model.
//!
//! The model describes a projection, not a universal programming-language AST,
//! and is deliberately free of language grammar and Git types.

use std::fmt;

use bstr::{BString, ByteSlice};

use crate::diagnostic::RepoPathError;

/// The MVP supports only [`ProjectionMode::Types`] and
/// [`ProjectionMode::Signatures`]; `Public` and `Full` are later product modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    Types,
    Signatures,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    Elm,
    Rust,
}

/// Must never carry a language AST node, a Tree-sitter node, or a `gix` handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemKind {
    Module,
    Type,
    TypeAlias,
    Constructor,
    Field,
    Variant,
    Trait,
    TraitImplementation,
    AssociatedType,
    Function,
    Method,
    Value,
    Constant,
    Static,
    Port,
    Operator,
    ForeignBlock,
}

/// Byte offsets are into the decoded UTF-8 source; line and column values are
/// zero-based.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedItem {
    /// A stable identity for the declaration within its projected file. Keys
    /// must not include byte offsets (TECHNICAL_DESIGN.md section 5.2).
    pub stable_key: String,
    /// The `stable_key` of the containing top-level declaration when this item
    /// is an **index-only** nested entry.
    ///
    /// An item with `parent_key == Some(_)` is not emitted separately by
    /// [`ProjectedFile`]: [`ProjectedItem::canonical_text`] already appears inside
    /// its ancestor's fragment. Emission is determined solely by this field, not
    /// by container naming in `stable_key`. Adapters must ensure the referenced
    /// key exists and that the item reaches a top-level ancestor.
    pub parent_key: Option<String>,
    pub kind: ItemKind,
    pub name: String,
    pub span: SourceSpan,
    pub canonical_text: String,
}

/// A projected source file and its declaration list.
///
/// # Canonical text
///
/// [`ProjectedFile::canonical_text`] is a pure function of `items`, computed by
/// [`ProjectedFile::new`], and is never independently mutable. The fields are
/// private and there is intentionally no mutable item accessor. Emission is
/// determined solely by [`ProjectedItem::parent_key`]:
///
/// 1. Only items with `parent_key == None` are emitted. Their canonical-text
///    fragments are joined by exactly one blank line, and the result ends with
///    exactly one trailing newline. A file with no top-level items has empty
///    canonical text.
/// 2. An item with `parent_key == Some(_)` is an index-only entry. Its text is
///    already contained in the fragment of the ancestor that owns it, so it is
///    never emitted as its own top-level block.
/// 3. Every item fragment is self-contained. A top-level fragment includes the
///    canonical rendering of its nested members, indented four spaces per
///    nesting level. A nested item's text appears exactly once in the file text.
/// 4. The `parent_key` relationship is independent of the container naming used
///    in stable keys. Naming a container in a stable key does not suppress
///    emission; only `parent_key == Some(_)` does. Adapters mark declarations
///    they want emitted as top-level.
///
/// # Adapter obligation
///
/// Every nested item must have a top-level ancestor, and an adapter must not
/// produce an item whose `parent_key` refers to a non-existent item. Core treats
/// this as an adapter obligation and does not validate it at runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedFile {
    path: RepoPath,
    language: Language,
    items: Vec<ProjectedItem>,
    canonical_text: String,
}

impl ProjectedFile {
    /// Derives canonical text from `items`; see [`ProjectedFile`] for the
    /// assembly contract.
    pub fn new(path: RepoPath, language: Language, items: Vec<ProjectedItem>) -> Self {
        let canonical_text = derive_canonical_text(&items);
        Self {
            path,
            language,
            items,
            canonical_text,
        }
    }

    pub fn path(&self) -> &RepoPath {
        &self.path
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn items(&self) -> &[ProjectedItem] {
        &self.items
    }

    pub fn canonical_text(&self) -> &str {
        &self.canonical_text
    }
}

fn derive_canonical_text(items: &[ProjectedItem]) -> String {
    let mut text = String::new();
    let mut emitted = false;
    for item in items.iter().filter(|item| item.parent_key.is_none()) {
        if emitted {
            text.push_str("\n\n");
        }
        text.push_str(item.canonical_text.trim_end_matches('\n'));
        emitted = true;
    }

    if emitted {
        text.push('\n');
    }
    text
}

/// A normalized repository-relative path.
///
/// Git paths are byte strings, not guaranteed UTF-8 operating-system paths, so
/// `RepoPath` owns the raw bytes and uses `/` as the separator. It sorts by raw
/// path bytes for deterministic output and must not be converted to a
/// [`std::path::PathBuf`] merely to inspect a committed tree.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RepoPath(BString);

impl RepoPath {
    pub fn new(bytes: impl AsRef<[u8]>) -> Result<Self, RepoPathError> {
        let bytes = bytes.as_ref();

        if bytes.is_empty() {
            return Err(RepoPathError::Empty);
        }
        if bytes[0] == b'/' {
            return Err(RepoPathError::Absolute);
        }

        for component in bytes.split(|&byte| byte == b'/') {
            if component == b".." {
                return Err(RepoPathError::ParentTraversal);
            }
            if component.is_empty() || component == b"." || component.contains(&0) {
                return Err(RepoPathError::NotNormalized);
            }
        }

        Ok(Self(BString::from(bytes.to_vec())))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// The file extension bytes after the final `.` in the final component;
    /// `None` for hidden files (leading `.`) and names ending in `.`.
    pub fn extension(&self) -> Option<&[u8]> {
        let bytes = self.0.as_slice();
        let file_name_start = bytes
            .iter()
            .rposition(|&byte| byte == b'/')
            .map_or(0, |index| index + 1);
        let file_name = &bytes[file_name_start..];

        let dot = file_name.iter().rposition(|&byte| byte == b'.')?;
        if dot == 0 || dot + 1 == file_name.len() {
            return None;
        }
        Some(&file_name[dot + 1..])
    }

    pub fn is_elm(&self) -> bool {
        self.extension() == Some(b"elm")
    }

    pub fn is_rust(&self) -> bool {
        self.extension() == Some(b"rs")
    }

    /// Extension matching is case-sensitive ASCII bytes.
    pub fn language(&self) -> Option<Language> {
        if self.is_elm() {
            Some(Language::Elm)
        } else if self.is_rust() {
            Some(Language::Rust)
        } else {
            None
        }
    }
}

impl fmt::Display for RepoPath {
    /// Escapes invalid UTF-8 bytes as `\xNN`; valid UTF-8, including spaces, is
    /// preserved. This never panics.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in ByteSlice::utf8_chunks(self.0.as_slice()) {
            f.write_str(chunk.valid())?;
            for &byte in chunk.invalid() {
                write!(f, "\\x{byte:02X}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, canonical_text: &str) -> ProjectedItem {
        item_with_parent(key, None, canonical_text)
    }

    fn item_with_parent(
        key: &str,
        parent_key: Option<&str>,
        canonical_text: &str,
    ) -> ProjectedItem {
        ProjectedItem {
            stable_key: key.to_owned(),
            parent_key: parent_key.map(str::to_owned),
            kind: ItemKind::Function,
            name: key.to_owned(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: 0,
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
            },
            canonical_text: canonical_text.to_owned(),
        }
    }

    #[test]
    fn repository_paths_reject_absolute_paths() {
        assert_eq!(RepoPath::new("/etc/passwd"), Err(RepoPathError::Absolute));
        assert_eq!(RepoPath::new("/"), Err(RepoPathError::Absolute));
    }

    #[test]
    fn repository_paths_reject_parent_traversal() {
        assert_eq!(
            RepoPath::new("../Cargo.toml"),
            Err(RepoPathError::ParentTraversal)
        );
        assert_eq!(
            RepoPath::new("src/../lib.rs"),
            Err(RepoPathError::ParentTraversal)
        );
        assert_eq!(RepoPath::new("src/.."), Err(RepoPathError::ParentTraversal));
    }

    #[test]
    fn repository_paths_reject_non_normalized_forms() {
        assert_eq!(RepoPath::new(""), Err(RepoPathError::Empty));
        assert_eq!(
            RepoPath::new("src//lib.rs"),
            Err(RepoPathError::NotNormalized)
        );
        assert_eq!(
            RepoPath::new("./src/lib.rs"),
            Err(RepoPathError::NotNormalized)
        );
        assert_eq!(
            RepoPath::new("src/lib.rs/"),
            Err(RepoPathError::NotNormalized)
        );
    }

    #[test]
    fn repository_paths_sort_by_raw_bytes() {
        let mut paths = [
            RepoPath::new("b.rs").unwrap(),
            RepoPath::new("a/x.rs").unwrap(),
            RepoPath::new("a.rs").unwrap(),
        ];
        paths.sort();
        let displayed: Vec<String> = paths.iter().map(ToString::to_string).collect();
        assert_eq!(displayed, vec!["a.rs", "a/x.rs", "b.rs"]);
    }

    #[test]
    fn repository_paths_detect_supported_extensions() {
        assert_eq!(
            RepoPath::new("src/User.elm").unwrap().language(),
            Some(Language::Elm)
        );
        assert_eq!(
            RepoPath::new("src/lib.rs").unwrap().language(),
            Some(Language::Rust)
        );
        assert_eq!(
            RepoPath::new("src/User.elm").unwrap().extension(),
            Some(b"elm".as_slice())
        );
        assert_eq!(RepoPath::new("README.md").unwrap().language(), None);
        assert_eq!(RepoPath::new(".gitignore").unwrap().extension(), None);
        assert_eq!(RepoPath::new("src/Foo.RS").unwrap().language(), None);
        assert_eq!(RepoPath::new("src/noext").unwrap().language(), None);
        assert_eq!(RepoPath::new("src/trailing.").unwrap().extension(), None);
    }

    #[test]
    fn repository_path_display_escapes_invalid_utf8() {
        let path = RepoPath::new(b"src/\xFF/lib.rs").unwrap();
        let displayed = path.to_string();
        assert!(
            displayed.contains(r"\xFF"),
            "expected escaped invalid byte in {displayed:?}"
        );
        assert!(displayed.ends_with("lib.rs"));
    }

    #[test]
    fn repository_path_display_preserves_valid_utf8() {
        let path = RepoPath::new("src/hello world.rs").unwrap();
        assert_eq!(path.to_string(), "src/hello world.rs");
        let unicode = RepoPath::new("src/☃.rs").unwrap();
        assert_eq!(unicode.to_string(), "src/☃.rs");
    }

    #[test]
    fn projected_file_separates_top_level_items_with_one_blank_line() {
        let file = ProjectedFile::new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item("a", "pub fn a();"), item("b", "pub fn b();")],
        );
        assert_eq!(file.canonical_text(), "pub fn a();\n\npub fn b();\n");
        assert_eq!(file.items().len(), 2);
        assert_eq!(file.path().to_string(), "src/lib.rs");
        assert_eq!(file.language(), Language::Rust);
    }

    #[test]
    fn projected_file_does_not_emit_nested_items_separately() {
        let top = item_with_parent("impl User", None, "impl User {\n    fn id(&self);\n}");
        let nested = item_with_parent("impl User::id", Some("impl User"), "    fn id(&self);");

        let file = ProjectedFile::new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![top, nested],
        );

        // The nested fragment appears once, inside its ancestor, not as its own
        // block.
        assert_eq!(file.canonical_text(), "impl User {\n    fn id(&self);\n}\n");
        assert_eq!(file.canonical_text().matches("fn id(&self);").count(), 1);
        assert_eq!(file.items().len(), 2);
    }

    #[test]
    fn projected_file_normalizes_trailing_newlines() {
        let file = ProjectedFile::new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item("a", "pub fn a();\n\n\n")],
        );
        assert_eq!(file.canonical_text(), "pub fn a();\n");
    }

    #[test]
    fn projected_file_with_no_items_has_empty_text() {
        let file = ProjectedFile::new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            Vec::new(),
        );
        assert!(file.items().is_empty());
        assert_eq!(file.canonical_text(), "");
    }

    #[test]
    fn projected_file_with_only_nested_items_has_empty_text() {
        let file = ProjectedFile::new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item_with_parent(
                "impl User::id",
                Some("impl User"),
                "    fn id(&self);",
            )],
        );
        assert_eq!(file.items().len(), 1);
        assert_eq!(file.canonical_text(), "");
    }
}
