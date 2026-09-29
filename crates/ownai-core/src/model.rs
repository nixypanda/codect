//! The shared projection model.
//!
//! The model describes a projection, not a universal programming-language AST,
//! and is deliberately free of language grammar and Git types.

use std::collections::{HashMap, HashSet};
use std::fmt;

use bstr::{BString, ByteSlice};

use crate::diagnostic::{ProjectionError, RepoPathError};

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
    Haskell,
    Python,
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
    TypeFamily,
    PatternSynonym,
    Function,
    Method,
    Value,
    Constant,
    Static,
    Port,
    Operator,
    ForeignBlock,
}

/// A half-open byte range and its zero-based start/end line-column positions.
///
/// Byte offsets index the decoded UTF-8 source; line and column values are
/// zero-based. `start_byte <= end_byte` and the start position is not after the
/// end position; [`SourceSpan::new`] establishes this.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    start_byte: usize,
    end_byte: usize,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

impl SourceSpan {
    /// All positions must come from one parse of the same source.
    pub fn new(
        start_byte: usize,
        end_byte: usize,
        start_line: usize,
        start_column: usize,
        end_line: usize,
        end_column: usize,
    ) -> Self {
        debug_assert!(start_byte <= end_byte);
        debug_assert!((start_line, start_column) <= (end_line, end_column));
        Self {
            start_byte,
            end_byte,
            start_line,
            start_column,
            end_line,
            end_column,
        }
    }

    pub fn start_byte(&self) -> usize {
        self.start_byte
    }

    pub fn end_byte(&self) -> usize {
        self.end_byte
    }

    pub fn start_line(&self) -> usize {
        self.start_line
    }

    pub fn start_column(&self) -> usize {
        self.start_column
    }

    pub fn end_line(&self) -> usize {
        self.end_line
    }

    pub fn end_column(&self) -> usize {
        self.end_column
    }
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
/// [`ProjectedFile::try_new`], and is never independently mutable. The fields
/// are private and there is intentionally no mutable item accessor. Emission is
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
/// 5. Every item reaches a top-level ancestor: the constructor rejects a
///    dangling, self-referential, or cyclic `parent_key`, and a duplicate
///    `stable_key`, so canonical-text assembly cannot silently drop an item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedFile {
    path: RepoPath,
    language: Language,
    items: Vec<ProjectedItem>,
    canonical_text: String,
}

impl ProjectedFile {
    /// Builds a projected file, validating the `parent_key` forest and the
    /// `path`/`language` pairing.
    ///
    /// Rejects a duplicate `stable_key`, a `parent_key` with no matching item,
    /// a self-parent, a cycle, an unsupported extension, and a `language` that
    /// disagrees with `path`. On success every nested item has a top-level
    /// ancestor, so no item is silently dropped by canonical-text assembly.
    ///
    /// A failure here means an adapter produced a value it should not have; see
    /// [`ProjectionError::AstInvariant`].
    pub fn try_new(
        path: RepoPath,
        language: Language,
        items: Vec<ProjectedItem>,
    ) -> Result<Self, ProjectionError> {
        let range = items
            .first()
            .map(|item| item.span)
            .unwrap_or_else(empty_span);
        let Some(derived) = path.language() else {
            return Err(ast_invariant(
                path,
                language,
                range,
                "path has no supported source extension",
            ));
        };
        if derived != language {
            return Err(ast_invariant(
                path,
                language,
                range,
                format!("path maps to {derived:?}, not {language:?}"),
            ));
        }

        // A duplicate key makes outline retention and per-declaration addressing
        // ambiguous, so the item list must name each declaration once.
        let mut index: HashMap<&str, usize> = HashMap::with_capacity(items.len());
        for (position, item) in items.iter().enumerate() {
            if index.insert(item.stable_key.as_str(), position).is_some() {
                return Err(ast_invariant(
                    path,
                    language,
                    item.span,
                    format!("duplicate stable key `{}`", item.stable_key),
                ));
            }
        }

        // Walk every ancestor chain to top level. Missing keys, self-parents,
        // and revisits are all fatal, and together they guarantee a top-level
        // ancestor exists for every item.
        for item in &items {
            let mut current = item.parent_key.as_deref();
            let mut visited = HashSet::new();
            while let Some(parent) = current {
                if parent == item.stable_key {
                    return Err(ast_invariant(
                        path,
                        language,
                        item.span,
                        format!("`{}` is its own parent", item.stable_key),
                    ));
                }
                if !visited.insert(parent) {
                    return Err(ast_invariant(
                        path,
                        language,
                        item.span,
                        format!("`parent_key` chain for `{}` is cyclic", item.stable_key),
                    ));
                }
                let Some(&position) = index.get(parent) else {
                    return Err(ast_invariant(
                        path,
                        language,
                        item.span,
                        format!("`{}` refers to unknown parent `{parent}`", item.stable_key),
                    ));
                };
                current = items[position].parent_key.as_deref();
            }
        }

        Ok(Self::assemble(path, language, items))
    }

    /// Assembles a file whose invariants are already established.
    fn assemble(path: RepoPath, language: Language, items: Vec<ProjectedItem>) -> Self {
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

/// A zero-width span used when an invariant failure is not tied to one item.
fn empty_span() -> SourceSpan {
    SourceSpan::new(0, 0, 0, 0, 0, 0)
}

fn ast_invariant(
    path: RepoPath,
    language: Language,
    range: SourceSpan,
    detail: impl Into<String>,
) -> ProjectionError {
    ProjectionError::AstInvariant {
        path,
        language,
        range,
        detail: detail.into(),
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

    /// Standard Haskell source (`.hs`); literate Haskell (`.lhs`) is not
    /// supported because the grammar cannot parse it.
    pub fn is_haskell(&self) -> bool {
        self.extension() == Some(b"hs")
    }

    /// Python source (`.py`) and type stubs (`.pyi`).
    pub fn is_python(&self) -> bool {
        matches!(self.extension(), Some(b"py" | b"pyi"))
    }

    /// Extension matching is case-sensitive ASCII bytes.
    pub fn language(&self) -> Option<Language> {
        if self.is_elm() {
            Some(Language::Elm)
        } else if self.is_rust() {
            Some(Language::Rust)
        } else if self.is_haskell() {
            Some(Language::Haskell)
        } else if self.is_python() {
            Some(Language::Python)
        } else {
            None
        }
    }

    /// True when `self` is `dir` itself or a descendant path under it. The check
    /// is byte-exact and boundary-aware, so `src` does not contain `src2/x.rs`.
    pub fn is_within(&self, dir: &RepoPath) -> bool {
        let path = self.as_bytes();
        let dir = dir.as_bytes();
        path == dir || (path.len() > dir.len() && path[dir.len()] == b'/' && path.starts_with(dir))
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
            span: SourceSpan::new(0, 0, 0, 0, 0, 0),
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
            RepoPath::new("src/Main.hs").unwrap().language(),
            Some(Language::Haskell)
        );
        assert_eq!(
            RepoPath::new("src/app.py").unwrap().language(),
            Some(Language::Python)
        );
        assert_eq!(
            RepoPath::new("src/app.pyi").unwrap().language(),
            Some(Language::Python)
        );
        assert_eq!(
            RepoPath::new("src/User.elm").unwrap().extension(),
            Some(b"elm".as_slice())
        );
        assert_eq!(RepoPath::new("README.md").unwrap().language(), None);
        assert_eq!(RepoPath::new(".gitignore").unwrap().extension(), None);
        assert_eq!(RepoPath::new("src/Foo.RS").unwrap().language(), None);
        assert_eq!(RepoPath::new("src/Foo.HS").unwrap().language(), None);
        assert_eq!(RepoPath::new("src/Foo.PY").unwrap().language(), None);
        assert_eq!(RepoPath::new("src/Main.lhs").unwrap().language(), None);
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
    fn repository_path_is_within_includes_itself() {
        let dir = RepoPath::new("src").unwrap();
        assert!(dir.is_within(&dir));
    }

    #[test]
    fn repository_path_is_within_includes_descendants() {
        let dir = RepoPath::new("src").unwrap();
        assert!(RepoPath::new("src/lib.rs").unwrap().is_within(&dir));
        assert!(
            RepoPath::new("src/nested/deep/lib.rs")
                .unwrap()
                .is_within(&dir)
        );
    }

    #[test]
    fn repository_path_is_within_respects_component_boundaries() {
        let dir = RepoPath::new("src").unwrap();
        assert!(!RepoPath::new("src2/x.rs").unwrap().is_within(&dir));
        assert!(!RepoPath::new("srclib.rs").unwrap().is_within(&dir));
    }

    #[test]
    fn repository_path_is_within_is_direction_sensitive() {
        let dir = RepoPath::new("src").unwrap();
        let descendant = RepoPath::new("src/lib.rs").unwrap();
        assert!(!dir.is_within(&descendant));
    }

    #[test]
    fn projected_file_separates_top_level_items_with_one_blank_line() {
        let file = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item("a", "pub fn a();"), item("b", "pub fn b();")],
        )
        .expect("valid fixture");
        assert_eq!(file.canonical_text(), "pub fn a();\n\npub fn b();\n");
        assert_eq!(file.items().len(), 2);
        assert_eq!(file.path().to_string(), "src/lib.rs");
        assert_eq!(file.language(), Language::Rust);
    }

    #[test]
    fn projected_file_does_not_emit_nested_items_separately() {
        let top = item_with_parent("impl User", None, "impl User {\n    fn id(&self);\n}");
        let nested = item_with_parent("impl User::id", Some("impl User"), "    fn id(&self);");

        let file = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![top, nested],
        )
        .expect("valid fixture");

        // The nested fragment appears once, inside its ancestor, not as its own
        // block.
        assert_eq!(file.canonical_text(), "impl User {\n    fn id(&self);\n}\n");
        assert_eq!(file.canonical_text().matches("fn id(&self);").count(), 1);
        assert_eq!(file.items().len(), 2);
    }

    #[test]
    fn projected_file_normalizes_trailing_newlines() {
        let file = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item("a", "pub fn a();\n\n\n")],
        )
        .expect("valid fixture");
        assert_eq!(file.canonical_text(), "pub fn a();\n");
    }

    #[test]
    fn projected_file_with_no_items_has_empty_text() {
        let file = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            Vec::new(),
        )
        .expect("valid fixture");
        assert!(file.items().is_empty());
        assert_eq!(file.canonical_text(), "");
    }

    #[test]
    fn projected_file_rejects_a_dangling_parent_key() {
        let error = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item_with_parent(
                "impl User::id",
                Some("impl User"),
                "    fn id(&self);",
            )],
        )
        .expect_err("a nested item with no container must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }

    #[test]
    fn projected_file_rejects_a_self_parent() {
        let error = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item_with_parent(
                "impl User",
                Some("impl User"),
                "impl User {\n}",
            )],
        )
        .expect_err("an item that is its own parent must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }

    #[test]
    fn projected_file_rejects_a_parent_cycle() {
        let error = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![
                item_with_parent("a", Some("b"), "a"),
                item_with_parent("b", Some("a"), "b"),
            ],
        )
        .expect_err("a cyclic parent chain must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }

    #[test]
    fn projected_file_rejects_a_duplicate_stable_key() {
        let error = ProjectedFile::try_new(
            RepoPath::new("src/lib.rs").unwrap(),
            Language::Rust,
            vec![item("a", "pub fn a();"), item("a", "pub fn a();")],
        )
        .expect_err("a duplicate stable key must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }

    #[test]
    fn projected_file_rejects_a_path_language_mismatch() {
        let error = ProjectedFile::try_new(
            RepoPath::new("src/app.py").unwrap(),
            Language::Rust,
            vec![item("a", "def a(): ...")],
        )
        .expect_err("a language that disagrees with the extension must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }

    #[test]
    fn projected_file_rejects_an_unsupported_extension() {
        let error = ProjectedFile::try_new(
            RepoPath::new("README.md").unwrap(),
            Language::Rust,
            Vec::new(),
        )
        .expect_err("an unsupported extension must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }

    #[test]
    fn source_span_accessors_return_the_constructed_positions() {
        let span = SourceSpan::new(4, 12, 1, 4, 2, 3);
        assert_eq!(span.start_byte(), 4);
        assert_eq!(span.end_byte(), 12);
        assert_eq!(span.start_line(), 1);
        assert_eq!(span.start_column(), 4);
        assert_eq!(span.end_line(), 2);
        assert_eq!(span.end_column(), 3);
    }
}
