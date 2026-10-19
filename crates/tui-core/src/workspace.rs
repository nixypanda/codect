//! The workspace: the committed query, the shared file set, the selection, and
//! the search.
//!
//! The kernel owns this; plugins read the selection and the committed query
//! through [`crate::cx::Cx`].

use std::collections::BTreeMap;
use std::sync::Arc;

use base::{ProjectionMode, RepoPath};

use crate::search::{Search, SearchPane, search_ranges};

/// A scope: the selection and a human label for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scope {
    pub selection: base::Selection,
    pub label: String,
}

impl Scope {
    pub fn new(selection: base::Selection) -> Self {
        let label = if selection.groups().is_empty() {
            "all".to_owned()
        } else {
            selection
                .groups()
                .iter()
                .map(base::SelectionGroup::label)
                .collect::<Vec<_>>()
                .join(", ")
        };
        Self { selection, label }
    }
}

/// The committed projection query: the mode and scope the active view loaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query {
    pub mode: ProjectionMode,
    pub scope: Scope,
}

/// The file set, selection, committed query, and search the kernel owns for the
/// active session.
#[derive(Clone, Debug)]
pub struct Workspace {
    pub query: Query,
    pub files: FileSet,
    pub selected: Option<RepoPath>,
    pub search: Option<Search>,
}

/// The files the active view loaded and the tree's presentation of them.
#[derive(Clone, Debug, Default)]
pub struct FileSet {
    /// Raw path byte order, as loaded.
    pub paths: Arc<[RepoPath]>,
    /// The change badge each path shows in the tree.
    pub badges: Arc<BTreeMap<RepoPath, BadgeTone>>,
    /// The tree's empty state when there are no files.
    pub empty: EmptyText,
}

/// The tree's empty state: a title and a detail line.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyText {
    pub title: &'static str,
    pub detail: &'static str,
}

/// The tree tone for a changed file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BadgeTone {
    Added,
    Modified,
    Deleted,
}

/// The tree's empty text for a `show` session.
pub const NO_FILES: EmptyText = EmptyText {
    title: "No files",
    detail: "No source files in this scope.",
};

/// The tree's empty text for a `diff` session.
pub const NO_CHANGES: EmptyText = EmptyText {
    title: "No changes",
    detail: "Body-only edits are omitted.",
};

/// The projection modes, in picker order.
pub fn available_modes() -> [ProjectionMode; 3] {
    [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ]
}

/// The label a mode shows in the header and picker.
pub fn mode_label(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
        ProjectionMode::Tests => "tests",
    }
}

impl Workspace {
    /// The ranges to highlight on `line` in `pane`, each tagged with whether it
    /// holds the global search cursor.
    pub fn ranges_on_line(&self, pane: SearchPane, line: usize) -> Vec<(usize, usize, bool)> {
        search_ranges(self.search.as_ref(), pane, line)
    }
}
