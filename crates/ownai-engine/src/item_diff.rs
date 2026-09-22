//! Deterministic declaration-level differences derived from projected files.
//!
//! This layer compares projection items, not source syntax. It deliberately
//! ignores index-only nested items because their canonical text is already part
//! of the top-level declaration that owns them.

use std::collections::{BTreeMap, BTreeSet};

use ownai_core::{ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath};

use crate::engine::FileDiff;

/// How one top-level projected declaration changed between two revisions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemChangeKind {
    Added,
    Removed,
    Modified,
}

/// One deterministic top-level declaration difference.
///
/// Results are ordered by `stable_key` within a file. An absent text denotes
/// the side on which an addition or removal does not exist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemDiff {
    pub path: RepoPath,
    pub language: Language,
    pub mode: ProjectionMode,
    pub change: ItemChangeKind,
    pub stable_key: String,
    pub item_kind: ItemKind,
    pub name: String,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
}

impl FileDiff {
    /// Derives changed top-level declarations from this file comparison.
    ///
    /// Alignment uses stable keys. Equal canonical fragments are omitted, so a
    /// source-only change or declaration reorder does not become a declaration
    /// change. Nested index entries are omitted because the owning top-level
    /// fragment already contains their text.
    pub fn item_diffs(&self) -> Vec<ItemDiff> {
        let old = top_level_items(self.old.as_ref());
        let new = top_level_items(self.new.as_ref());
        let keys: BTreeSet<&str> = old.keys().chain(new.keys()).copied().collect();

        keys.into_iter()
            .filter_map(|stable_key| {
                build_item_diff(
                    &self.path,
                    language_of(self),
                    self.mode,
                    old.get(stable_key).copied(),
                    new.get(stable_key).copied(),
                )
            })
            .collect()
    }
}

fn top_level_items(file: Option<&ProjectedFile>) -> BTreeMap<&str, &ProjectedItem> {
    file.into_iter()
        .flat_map(ProjectedFile::items)
        .filter(|item| item.parent_key.is_none())
        .map(|item| (item.stable_key.as_str(), item))
        .collect()
}

fn language_of(diff: &FileDiff) -> Language {
    diff.new
        .as_ref()
        .or(diff.old.as_ref())
        .expect("a FileDiff has at least one projected side")
        .language()
}

fn build_item_diff(
    path: &RepoPath,
    language: Language,
    mode: ProjectionMode,
    old: Option<&ProjectedItem>,
    new: Option<&ProjectedItem>,
) -> Option<ItemDiff> {
    let (change, item) = match (old, new) {
        (None, Some(new)) => (ItemChangeKind::Added, new),
        (Some(old), None) => (ItemChangeKind::Removed, old),
        (Some(old), Some(new)) if old.canonical_text != new.canonical_text => {
            (ItemChangeKind::Modified, new)
        }
        (Some(_), Some(_)) => return None,
        (None, None) => unreachable!("the stable key came from at least one side"),
    };

    Some(ItemDiff {
        path: path.clone(),
        language,
        mode,
        change,
        stable_key: item.stable_key.clone(),
        item_kind: item.kind,
        name: item.name.clone(),
        old_text: old.map(|item| item.canonical_text.clone()),
        new_text: new.map(|item| item.canonical_text.clone()),
    })
}

#[cfg(test)]
mod tests {
    use ownai_core::{ProjectedFile, SourceSpan};

    use super::*;

    fn path() -> RepoPath {
        RepoPath::new("src/lib.rs").expect("valid path")
    }

    fn item(
        stable_key: &str,
        parent_key: Option<&str>,
        kind: ItemKind,
        text: &str,
    ) -> ProjectedItem {
        ProjectedItem {
            stable_key: stable_key.to_owned(),
            parent_key: parent_key.map(str::to_owned),
            kind,
            name: stable_key.to_owned(),
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

    fn file(items: Vec<ProjectedItem>) -> ProjectedFile {
        ProjectedFile::new(path(), Language::Rust, items)
    }

    fn diff(old: Option<ProjectedFile>, new: Option<ProjectedFile>) -> FileDiff {
        FileDiff {
            path: path(),
            mode: ProjectionMode::Types,
            old,
            new,
        }
    }

    #[test]
    fn aligns_items_by_stable_key_and_sorts_the_result() {
        let old = file(vec![
            item("z-removed", None, ItemKind::Function, "fn removed();"),
            item("b-same", None, ItemKind::Function, "fn same();"),
            item("m-modified", None, ItemKind::Function, "fn changed(u8);"),
        ]);
        let new = file(vec![
            item("a-added", None, ItemKind::Type, "struct Added;"),
            item("m-modified", None, ItemKind::Function, "fn changed(u16);"),
            item("b-same", None, ItemKind::Function, "fn same();"),
        ]);

        let items = diff(Some(old), Some(new)).item_diffs();

        assert_eq!(
            items
                .iter()
                .map(|item| (item.stable_key.as_str(), item.change))
                .collect::<Vec<_>>(),
            vec![
                ("a-added", ItemChangeKind::Added),
                ("m-modified", ItemChangeKind::Modified),
                ("z-removed", ItemChangeKind::Removed),
            ]
        );
        assert_eq!(items[0].old_text, None);
        assert_eq!(items[0].new_text.as_deref(), Some("struct Added;"));
        assert_eq!(items[1].old_text.as_deref(), Some("fn changed(u8);"));
        assert_eq!(items[1].new_text.as_deref(), Some("fn changed(u16);"));
        assert_eq!(items[2].old_text.as_deref(), Some("fn removed();"));
        assert_eq!(items[2].new_text, None);
    }

    #[test]
    fn ignores_nested_index_items() {
        let old = file(vec![
            item("type User", None, ItemKind::Type, "struct User { id: u8 }"),
            item(
                "type User::field id",
                Some("type User"),
                ItemKind::Field,
                "id: u8",
            ),
        ]);
        let new = file(vec![
            item("type User", None, ItemKind::Type, "struct User { id: u16 }"),
            item(
                "type User::field id",
                Some("type User"),
                ItemKind::Field,
                "id: u16",
            ),
        ]);

        let items = diff(Some(old), Some(new)).item_diffs();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].stable_key, "type User");
        assert_eq!(items[0].change, ItemChangeKind::Modified);
    }

    #[test]
    fn a_reorder_without_fragment_changes_is_not_an_item_change() {
        let first = item("function first", None, ItemKind::Function, "fn first();");
        let second = item("function second", None, ItemKind::Function, "fn second();");
        let old = file(vec![first.clone(), second.clone()]);
        let new = file(vec![second, first]);

        assert!(diff(Some(old), Some(new)).item_diffs().is_empty());
    }

    #[test]
    fn an_added_file_adds_each_top_level_item() {
        let new = file(vec![
            item("type User", None, ItemKind::Type, "struct User;"),
            item(
                "function load",
                None,
                ItemKind::Function,
                "fn load() -> User;",
            ),
        ]);

        let items = diff(None, Some(new)).item_diffs();

        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .all(|item| item.change == ItemChangeKind::Added && item.old_text.is_none())
        );
    }

    #[test]
    fn a_removed_file_removes_each_top_level_item() {
        let old = file(vec![
            item("type User", None, ItemKind::Type, "struct User;"),
            item(
                "function load",
                None,
                ItemKind::Function,
                "fn load() -> User;",
            ),
        ]);

        let items = diff(Some(old), None).item_diffs();

        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .all(|item| item.change == ItemChangeKind::Removed && item.new_text.is_none())
        );
    }
}
