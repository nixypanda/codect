//! The mode-independent declaration outline.
//!
//! The outline is derived from the Signatures projection, which is the superset
//! of the Types projection, so it can locate a declaration the requested mode
//! drops entirely (for example an inherent `impl` in Types mode). These types
//! carry no serialization dependency; the CLI owns the JSON form.

use std::collections::BTreeSet;

use crate::model::{ItemKind, Language, ProjectedFile, ProjectedItem, RepoPath, SourceSpan};

/// One declaration in a file's mode-independent outline.
///
/// The outline is derived from the Signatures projection, which is the superset
/// of the Types projection, so it can locate a declaration the requested mode
/// drops entirely (for example an inherent `impl` in Types mode).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutlineItem {
    pub stable_key: String,
    /// The `stable_key` of the containing declaration, or `None` at top level.
    pub parent_key: Option<String>,
    pub kind: ItemKind,
    pub name: String,
    /// The declaration's source span, as produced by the adapter (zero-based
    /// lines; the JSON layer converts them).
    pub span: SourceSpan,
    /// The declaration's canonical fragment from the Signatures projection.
    /// For a nested declaration this carries its container indentation.
    pub signature: String,
    /// Whether the requested mode's projection retains this `stable_key`.
    pub retained_in_mode: bool,
}

/// A requested-mode projection paired with the file's complete outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileOutline {
    pub path: RepoPath,
    pub language: Language,
    pub projection: ProjectedFile,
    pub outline: Vec<OutlineItem>,
}

/// One changed file with the complete declaration outline for each present side.
///
/// The variant says which sides exist, so a comparison with neither side, or a
/// path that disagrees with a present side, is not representable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileOutlineDiff {
    /// Only the new side is present.
    Added { new: FileOutline },
    /// Only the old side is present.
    Deleted { old: FileOutline },
    /// Both sides are present and differ in their requested-mode projection.
    Modified { old: FileOutline, new: FileOutline },
}

impl FileOutlineDiff {
    /// The compared file's path, taken from a present side so it can never
    /// disagree with the variant.
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::Added { new } => &new.path,
            Self::Deleted { old } => &old.path,
            Self::Modified { old, .. } => &old.path,
        }
    }
}

/// Builds a [`FileOutline`] from the requested-mode and Signatures projections.
///
/// `retained_in_mode` is set by `stable_key` membership in `projection`, so a
/// declaration the requested mode drops is still located through `superset`.
pub fn assemble_outline(
    path: &RepoPath,
    language: Language,
    projection: Vec<ProjectedItem>,
    superset: Vec<ProjectedItem>,
) -> FileOutline {
    let retained: BTreeSet<&str> = projection
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect();
    let outline = superset
        .iter()
        .map(|item| OutlineItem {
            stable_key: item.stable_key.clone(),
            parent_key: item.parent_key.clone(),
            kind: item.kind,
            name: item.name.clone(),
            span: item.span.clone(),
            signature: item.canonical_text.clone(),
            retained_in_mode: retained.contains(item.stable_key.as_str()),
        })
        .collect();
    FileOutline {
        path: path.clone(),
        language,
        projection: ProjectedFile::new(path.clone(), language, projection),
        outline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, parent: Option<&str>, text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: key.to_owned(),
            parent_key: parent.map(str::to_owned),
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
            canonical_text: text.to_owned(),
        }
    }

    #[test]
    fn outline_is_the_signatures_superset_with_retention_from_the_requested_mode() {
        let path = RepoPath::new("src/lib.rs").unwrap();
        let requested = vec![item("impl User", None, "impl User {\n}")];
        let superset = vec![
            item("impl User", None, "impl User {\n    fn id(&self);\n}"),
            item("impl User::id", Some("impl User"), "    fn id(&self);"),
        ];

        let file = assemble_outline(&path, Language::Rust, requested, superset);

        assert_eq!(file.projection.canonical_text(), "impl User {\n}\n");
        assert_eq!(file.outline.len(), 2);
        assert!(file.outline[0].retained_in_mode);
        assert!(
            !file.outline[1].retained_in_mode,
            "a declaration the requested mode drops must still appear in the outline"
        );
        assert_eq!(file.outline[1].signature, "    fn id(&self);");
    }
}
