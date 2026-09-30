//! The mode-independent declaration outline.
//!
//! The outline is derived from the Signatures projection, which is the superset
//! of the Types projection, so it can locate a declaration the requested mode
//! drops entirely (for example an inherent `impl` in Types mode). These types
//! carry no serialization dependency; the CLI owns the JSON form.

use std::collections::BTreeSet;

use crate::diagnostic::ProjectionError;
use crate::model::{Language, ProjectedFile, ProjectedItem, RepoPath, SupportedPath};

/// One declaration in a file's mode-independent outline.
///
/// The outline is derived from the Signatures projection, which is the superset
/// of the Types projection, so it can locate a declaration the requested mode
/// drops entirely (for example an inherent `impl` in Types mode). The item is
/// the Signatures-superset declaration itself, so it cannot disagree with the
/// projection it came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutlineItem {
    /// The declaration from the Signatures superset; `canonical_text` is the
    /// outline's `signature`.
    pub item: ProjectedItem,
}

/// A requested-mode projection paired with the file's complete outline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileOutline {
    pub projection: ProjectedFile,
    /// The `stable_key`s the requested mode retains.
    pub retained: BTreeSet<String>,
    /// The Signatures superset, in source order.
    pub outline: Vec<OutlineItem>,
}

impl FileOutline {
    /// The file's path, taken from the projection so it cannot disagree.
    pub fn path(&self) -> &RepoPath {
        self.projection.path()
    }

    /// The file's language, taken from the projection so it cannot disagree.
    pub fn language(&self) -> Language {
        self.projection.language()
    }

    /// Whether the requested mode's projection retains `item`'s `stable_key`.
    pub fn retained_in_mode(&self, item: &OutlineItem) -> bool {
        self.retained.contains(&item.item.stable_key)
    }
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
            Self::Added { new } => new.path(),
            Self::Deleted { old } => old.path(),
            Self::Modified { old, .. } => old.path(),
        }
    }
}

/// Builds a [`FileOutline`] from the requested-mode and Signatures projections.
///
/// Retention is `stable_key` membership in `projection`, so a declaration the
/// requested mode drops is still located through `superset`. Both projections
/// are validated, so the outline cannot carry a malformed parent forest.
pub fn assemble_outline(
    path: &SupportedPath,
    projection: Vec<ProjectedItem>,
    superset: Vec<ProjectedItem>,
) -> Result<FileOutline, ProjectionError> {
    let projection = ProjectedFile::try_new(path.clone(), projection)?;
    let superset = ProjectedFile::try_new(path.clone(), superset)?;
    let retained = projection
        .items()
        .iter()
        .map(|item| item.stable_key.clone())
        .collect();
    let outline = superset
        .items()
        .iter()
        .cloned()
        .map(|item| OutlineItem { item })
        .collect();
    Ok(FileOutline {
        projection,
        retained,
        outline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKind, SourceSpan, SupportedPath};

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    fn item(key: &str, parent: Option<&str>, text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: key.to_owned(),
            parent_key: parent.map(str::to_owned),
            kind: ItemKind::Function,
            name: key.to_owned(),
            span: SourceSpan::new(0, 0, 0, 0, 0, 0),
            canonical_text: text.to_owned(),
        }
    }

    #[test]
    fn outline_is_the_signatures_superset_with_retention_from_the_requested_mode() {
        let requested = vec![item("impl User", None, "impl User {\n}")];
        let superset = vec![
            item("impl User", None, "impl User {\n    fn id(&self);\n}"),
            item("impl User::id", Some("impl User"), "    fn id(&self);"),
        ];

        let file = assemble_outline(&supported("src/lib.rs"), requested, superset)
            .expect("valid outline fixture");

        assert_eq!(file.projection.canonical_text(), "impl User {\n}\n");
        assert_eq!(file.outline.len(), 2);
        assert!(file.retained_in_mode(&file.outline[0]));
        assert!(
            !file.retained_in_mode(&file.outline[1]),
            "a declaration the requested mode drops must still appear in the outline"
        );
        assert_eq!(file.outline[1].item.canonical_text, "    fn id(&self);");
        assert_eq!(file.path(), file.projection.path());
        assert_eq!(file.language(), file.projection.language());
    }
}
