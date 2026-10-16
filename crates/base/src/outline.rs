//! The mode-independent declaration outline.
//!
//! The outline is derived from the Signatures projection, which is the superset
//! of the Types projection, so it can locate a declaration the requested mode
//! drops entirely (for example an inherent `impl` in Types mode). These types
//! carry no serialization dependency; the CLI owns the JSON form.

use std::collections::BTreeSet;

use crate::diagnostic::ProjectionError;
use crate::model::{Language, ProjectedFile, ProjectedItem, RepoPath, SupportedPath};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutlineItem {
    pub item: ProjectedItem,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileOutline {
    pub projection: ProjectedFile,
    pub retained: BTreeSet<String>,
    pub outline: Vec<OutlineItem>,
}

impl FileOutline {
    pub fn path(&self) -> &RepoPath {
        self.projection.path()
    }

    pub fn language(&self) -> Language {
        self.projection.language()
    }

    pub fn retained_in_mode(&self, item: &OutlineItem) -> bool {
        self.retained.contains(&item.item.stable_key)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceOutline {
    pub outline: FileOutline,
    pub source: String,
}

impl SourceOutline {
    pub fn path(&self) -> &RepoPath {
        self.outline.path()
    }

    pub fn language(&self) -> Language {
        self.outline.language()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileOutlineDiff {
    Added { new: FileOutline },
    Deleted { old: FileOutline },
    Modified { old: FileOutline, new: FileOutline },
}

impl FileOutlineDiff {
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::Added { new } => new.path(),
            Self::Deleted { old } => old.path(),
            Self::Modified { old, .. } => old.path(),
        }
    }
}

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

    #[test]
    fn source_outline_delegates_path_and_language_to_its_outline() {
        let outline = assemble_outline(&supported("src/lib.rs"), Vec::new(), Vec::new())
            .expect("valid outline fixture");
        let source = SourceOutline {
            outline: outline.clone(),
            source: "pub fn main() {}\n".to_owned(),
        };

        assert_eq!(source.path(), outline.path());
        assert_eq!(source.language(), outline.language());
    }
}
