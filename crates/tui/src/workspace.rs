//! The workspace: the committed query, the shared file set, the selection, and
//! the search.
//!
//! This is the in-crate form of the plugin plan's section 4.6 `Workspace`. The
//! shell owns it; the pages read the selection and the committed query from it
//! through their callers and the tree renders from it directly.

use std::collections::BTreeMap;
use std::sync::Arc;

use base::{ProjectionMode, RepoPath};

use crate::page::Scope;

/// The file set, selection, committed query, and search the shell owns for the
/// active session.
#[derive(Clone, Debug)]
pub(crate) struct Workspace {
    pub query: Query,
    pub files: FileSet,
    pub selected: Option<RepoPath>,
    pub search: Option<Search>,
}

/// The committed projection query: the mode and scope the active view loaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Query {
    pub mode: ProjectionMode,
    pub scope: Scope,
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

/// The committed search: the needle, its matches in view order, and the cursor
/// that names the current match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Search {
    pub needle: String,
    pub matches: Vec<SearchMatch>,
    pub cursor: usize,
}

/// One search hit: the pane it belongs to and its byte range on a 1-based line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SearchMatch {
    pub pane: SearchPane,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// The pane a search match belongs to. Temporary until task 3.1 replaces it with
/// a `PaneId`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchPane {
    Source,
    Old,
    New,
}

impl Workspace {
    /// The ranges to highlight on `line` in `pane`, each tagged with whether it
    /// holds the global search cursor.
    pub(crate) fn ranges_on_line(
        &self,
        pane: SearchPane,
        line: usize,
    ) -> Vec<(usize, usize, bool)> {
        search_ranges(self.search.as_ref(), pane, line)
    }
}

/// Collect the case-insensitive, non-overlapping matches of `needle_lower` in
/// `text`, tagging each with `pane`. Lines are 1-based; `end` is the byte offset
/// of the character after the match, or the line length at the line's end.
pub(crate) fn collect_matches(
    text: &str,
    needle_lower: &str,
    pane: SearchPane,
    out: &mut Vec<SearchMatch>,
) {
    let needle: Vec<char> = needle_lower.chars().collect();
    if needle.is_empty() {
        return;
    }
    for (index, line) in text.lines().enumerate() {
        let hay: Vec<(usize, char)> = line.char_indices().collect();
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            let matched = (0..needle.len())
                .all(|k| hay[i + k].1.to_lowercase().next().unwrap_or(hay[i + k].1) == needle[k]);
            if matched {
                let start = hay[i].0;
                let end = hay
                    .get(i + needle.len())
                    .map_or(line.len(), |&(byte, _)| byte);
                out.push(SearchMatch {
                    pane,
                    line: index + 1,
                    start,
                    end,
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
}

/// The search ranges on one line and pane, tagged with whether each is the
/// global cursor's match.
pub(crate) fn search_ranges(
    search: Option<&Search>,
    pane: SearchPane,
    line: usize,
) -> Vec<(usize, usize, bool)> {
    let Some(search) = search else {
        return Vec::new();
    };
    search
        .matches
        .iter()
        .enumerate()
        .filter(|(_, matched)| matched.pane == pane && matched.line == line)
        .map(|(index, matched)| (matched.start, matched.end, index == search.cursor))
        .collect()
}

/// Advance the search cursor with wrap-around and return the new current match.
pub(crate) fn advance_search(search: &mut Option<Search>, forward: bool) -> Option<SearchMatch> {
    let search = search.as_mut()?;
    if search.matches.is_empty() {
        return None;
    }
    let len = search.matches.len();
    search.cursor = if forward {
        (search.cursor + 1) % len
    } else {
        (search.cursor + len - 1) % len
    };
    search.matches.get(search.cursor).cloned()
}

/// The match the search cursor currently names, if any.
pub(crate) fn current_match(search: &Option<Search>) -> Option<SearchMatch> {
    let search = search.as_ref()?;
    search.matches.get(search.cursor).cloned()
}
