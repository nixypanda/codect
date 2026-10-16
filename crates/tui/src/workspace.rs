//! The workspace: the shared file set and the selected path.
//!
//! This is the in-crate form of the plugin plan's section 4.6 `Workspace`. The
//! shell owns it; the pages read the selection from it through their callers and
//! the tree renders from it directly.

use std::collections::BTreeMap;
use std::sync::Arc;

use base::RepoPath;

/// The file set and selection the shell owns for the active session.
#[derive(Clone, Debug, Default)]
pub(crate) struct Workspace {
    pub files: FileSet,
    pub selected: Option<RepoPath>,
}

/// The files the active view loaded and the tree's presentation of them.
#[derive(Clone, Debug, Default)]
pub(crate) struct FileSet {
    /// Raw path byte order, as loaded.
    pub paths: Arc<[RepoPath]>,
    /// The change badge each path shows in the tree.
    pub badges: Arc<BTreeMap<RepoPath, BadgeTone>>,
    /// The tree's empty state when there are no files.
    pub empty: EmptyText,
}

/// The tree's empty state: a title and a detail line.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EmptyText {
    pub title: &'static str,
    pub detail: &'static str,
}

/// The tree tone for a changed file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BadgeTone {
    Added,
    Modified,
    Deleted,
}

/// The tree's empty text for a `show` session.
pub(crate) const NO_FILES: EmptyText = EmptyText {
    title: "No files",
    detail: "No source files in this scope.",
};

/// The tree's empty text for a `diff` session.
pub(crate) const NO_CHANGES: EmptyText = EmptyText {
    title: "No changes",
    detail: "Body-only edits are omitted.",
};
