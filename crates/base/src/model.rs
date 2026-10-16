//! The shared projection model.
//!
//! The model describes a projection, not a universal programming-language AST,
//! and is deliberately free of language grammar and Git types.
//!
//! Emission is determined solely by [`ProjectedItem::parent_key`]: `None` items
//! are emitted and `Some(_)` items are index-only, and [`ProjectedFile::try_new`]
//! rejects duplicate keys and a dangling, self-referential, or cyclic parent
//! chain. Stable keys never embed byte offsets, and repository paths sort by raw
//! bytes.
//!
//! Git paths are byte strings, not guaranteed UTF-8 operating-system paths, so
//! [`RepoPath`] owns the raw bytes and is never converted to a
//! [`std::path::PathBuf`] merely to inspect a committed tree.

use std::collections::{HashMap, HashSet};
use std::fmt;

use bstr::{BString, ByteSlice};

use crate::diagnostic::{ProjectionError, RepoPathError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    Types,
    Signatures,
    Tests,
}

impl ProjectionMode {
    pub fn superset(self) -> Self {
        match self {
            Self::Types | Self::Signatures => Self::Signatures,
            Self::Tests => Self::Signatures,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    Elm,
    Haskell,
    Python,
    Rust,
}

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
    pub stable_key: String,
    pub parent_key: Option<String>,
    pub kind: ItemKind,
    pub name: String,
    pub span: SourceSpan,
    pub canonical_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedFile {
    path: SupportedPath,
    items: Vec<ProjectedItem>,
    canonical_text: String,
}

impl ProjectedFile {
    pub fn try_new(
        path: SupportedPath,
        items: Vec<ProjectedItem>,
    ) -> Result<Self, ProjectionError> {
        let mut index: HashMap<&str, usize> = HashMap::with_capacity(items.len());
        for (position, item) in items.iter().enumerate() {
            if index.insert(item.stable_key.as_str(), position).is_some() {
                return Err(ast_invariant(
                    path.clone(),
                    item.span,
                    format!("duplicate stable key `{}`", item.stable_key),
                ));
            }
        }

        for item in &items {
            let mut current = item.parent_key.as_deref();
            let mut visited = HashSet::new();
            while let Some(parent) = current {
                if parent == item.stable_key {
                    return Err(ast_invariant(
                        path.clone(),
                        item.span,
                        format!("`{}` is its own parent", item.stable_key),
                    ));
                }
                if !visited.insert(parent) {
                    return Err(ast_invariant(
                        path.clone(),
                        item.span,
                        format!("`parent_key` chain for `{}` is cyclic", item.stable_key),
                    ));
                }
                let Some(&position) = index.get(parent) else {
                    return Err(ast_invariant(
                        path.clone(),
                        item.span,
                        format!("`{}` refers to unknown parent `{parent}`", item.stable_key),
                    ));
                };
                current = items[position].parent_key.as_deref();
            }
        }

        Ok(Self::assemble(path, items))
    }

    fn assemble(path: SupportedPath, items: Vec<ProjectedItem>) -> Self {
        let canonical_text = derive_canonical_text(&items);
        Self {
            path,
            items,
            canonical_text,
        }
    }

    pub fn path(&self) -> &RepoPath {
        self.path.path()
    }

    pub fn language(&self) -> Language {
        self.path.language()
    }

    pub fn items(&self) -> &[ProjectedItem] {
        &self.items
    }

    pub fn canonical_text(&self) -> &str {
        &self.canonical_text
    }
}

fn ast_invariant(
    path: SupportedPath,
    range: SourceSpan,
    detail: impl Into<String>,
) -> ProjectionError {
    ProjectionError::AstInvariant {
        path,
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

    pub fn is_python(&self) -> bool {
        matches!(self.extension(), Some(b"py" | b"pyi"))
    }

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

    pub fn is_within(&self, dir: &RepoPath) -> bool {
        let path = self.as_bytes();
        let dir = dir.as_bytes();
        path == dir || (path.len() > dir.len() && path[dir.len()] == b'/' && path.starts_with(dir))
    }
}

impl fmt::Display for RepoPath {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupportedPath {
    path: RepoPath,
    language: Language,
}

impl SupportedPath {
    pub fn new(path: RepoPath) -> Option<Self> {
        path.language().map(|language| Self { path, language })
    }

    pub fn path(&self) -> &RepoPath {
        &self.path
    }

    pub fn language(&self) -> Language {
        self.language
    }
}

impl fmt::Display for SupportedPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.fmt(f)
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

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    #[test]
    fn supported_path_wraps_a_supported_file_and_rejects_the_rest() {
        let path = supported("src/lib.rs");
        assert_eq!(path.path().to_string(), "src/lib.rs");
        assert_eq!(path.language(), Language::Rust);
        assert!(SupportedPath::new(RepoPath::new("README.md").unwrap()).is_none());
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
            supported("src/lib.rs"),
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

        let file = ProjectedFile::try_new(supported("src/lib.rs"), vec![top, nested])
            .expect("valid fixture");

        assert_eq!(file.canonical_text(), "impl User {\n    fn id(&self);\n}\n");
        assert_eq!(file.canonical_text().matches("fn id(&self);").count(), 1);
        assert_eq!(file.items().len(), 2);
    }

    #[test]
    fn projected_file_normalizes_trailing_newlines() {
        let file = ProjectedFile::try_new(
            supported("src/lib.rs"),
            vec![item("a", "pub fn a();\n\n\n")],
        )
        .expect("valid fixture");
        assert_eq!(file.canonical_text(), "pub fn a();\n");
    }

    #[test]
    fn projected_file_with_no_items_has_empty_text() {
        let file =
            ProjectedFile::try_new(supported("src/lib.rs"), Vec::new()).expect("valid fixture");
        assert!(file.items().is_empty());
        assert_eq!(file.canonical_text(), "");
    }

    #[test]
    fn projected_file_rejects_a_dangling_parent_key() {
        let error = ProjectedFile::try_new(
            supported("src/lib.rs"),
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
            supported("src/lib.rs"),
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
            supported("src/lib.rs"),
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
            supported("src/lib.rs"),
            vec![item("a", "pub fn a();"), item("a", "pub fn a();")],
        )
        .expect_err("a duplicate stable key must be rejected");
        assert!(matches!(error, ProjectionError::AstInvariant { .. }));
    }
}
