//! The diff plugin's model: the range or history view, its diffs, highlight
//! cache, and wrapped-row cache.
//!
//! The wrapped rows are keyed on a generation counter plus the layout inputs,
//! so any change to the diffs must bump the generation.

use std::sync::Arc;

use base::{FileDiff, ProjectedFile, RepoPath};

use tui_core::cx::Cx;
use tui_core::render::highlight::{self, StyledLine};
use tui_core::render::theme::Theme;
use tui_core::search::{SearchMatch, SearchPane, collect_matches};
use tui_core::util::cache::BoundedCache;
use tui_core::workspace::Workspace;

use super::commit_picker::CommitPicker;
use super::layout::{VisualRow, layout_diff, side_widths};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffBody {
    pub scroll: u16,
}

#[derive(Clone, Default)]
pub struct DiffHighlight {
    pub old: Vec<StyledLine>,
    pub new: Vec<StyledLine>,
}

#[derive(Clone, Debug, Default)]
pub struct DiffHighlights {
    cache: BoundedCache<Arc<DiffHighlight>>,
}

impl DiffHighlights {
    pub fn highlight(&self, path: &RepoPath) -> Option<&DiffHighlight> {
        self.cache.get(path).map(Arc::as_ref)
    }

    pub fn contains(&self, path: &RepoPath) -> bool {
        self.cache.contains_key(path)
    }

    fn insert(&mut self, path: RepoPath, highlight: DiffHighlight) {
        self.cache.insert(path, Arc::new(highlight));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[derive(Clone, Debug)]
pub enum DiffViewState {
    Range(Range),
    Commits(Commits),
}

#[derive(Clone, Debug)]
pub struct Range {
    pub body: DiffBody,
    pub diffs: Arc<[FileDiff]>,
    pub highlight: DiffHighlights,
}

#[derive(Clone, Debug)]
pub struct Commits {
    pub picker: CommitPicker,
    pub body: DiffBody,
    pub diffs: Arc<[FileDiff]>,
    pub highlight: DiffHighlights,
}

impl Range {
    pub fn empty() -> Self {
        Self {
            body: DiffBody { scroll: 0 },
            diffs: Arc::from(Vec::new()),
            highlight: DiffHighlights::default(),
        }
    }
}

impl Commits {
    pub fn empty() -> Self {
        Self {
            picker: CommitPicker::empty(),
            body: DiffBody { scroll: 0 },
            diffs: Arc::from(Vec::new()),
            highlight: DiffHighlights::default(),
        }
    }
}

/// The wrapped rows and the inputs they were built from.
#[derive(Clone, Debug, Default)]
pub struct DiffRows {
    key: Option<RowsKey>,
    rows: Vec<VisualRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RowsKey {
    generation: u64,
    width: u16,
    height: u16,
    selected: Option<RepoPath>,
    tree_percent: u16,
}

impl super::DiffPlugin {
    pub fn rows(&self) -> &[VisualRow] {
        self.rows.rows.as_slice()
    }

    pub fn diffs(&self) -> &[FileDiff] {
        match &self.view {
            DiffViewState::Range(range) => &range.diffs,
            DiffViewState::Commits(commits) => &commits.diffs,
        }
    }

    pub fn active_diff(&self, selected: Option<&RepoPath>) -> Option<&FileDiff> {
        let path = selected?;
        self.diffs().iter().find(|diff| diff.path() == path)
    }

    pub fn body_scroll(&self) -> u16 {
        match &self.view {
            DiffViewState::Range(range) => range.body.scroll,
            DiffViewState::Commits(commits) => commits.body.scroll,
        }
    }

    pub fn set_body_scroll(&mut self, scroll: u16) {
        match &mut self.view {
            DiffViewState::Range(range) => range.body.scroll = scroll,
            DiffViewState::Commits(commits) => commits.body.scroll = scroll,
        }
    }

    pub fn revisions(&self) -> (String, String) {
        if let DiffViewState::Commits(commits) = &self.view
            && let Some(step) = commits.picker.steps_slice().get(commits.picker.cursor())
        {
            let parent = step.parent_id.to_string();
            let commit = step.commit_id.to_string();
            return (
                parent[..parent.len().min(7)].to_owned(),
                commit[..commit.len().min(7)].to_owned(),
            );
        }
        (self.base.clone(), self.target.clone())
    }

    /// The search matches for `needle` over the active file diff. An added file
    /// matches on the new side; a deleted file on the old; a modified file on
    /// the old side first, then the new.
    pub fn search_matches(&self, needle: &str, workspace: &Workspace) -> Vec<SearchMatch> {
        let needle_lower = needle.to_lowercase();
        let mut matches = Vec::new();
        let Some(diff) = self.active_diff(workspace.selected.as_ref()) else {
            return matches;
        };
        match diff {
            FileDiff::Added { new } => collect_matches(
                new.canonical_text(),
                &needle_lower,
                SearchPane::New,
                &mut matches,
            ),
            FileDiff::Deleted { old } => collect_matches(
                old.canonical_text(),
                &needle_lower,
                SearchPane::Old,
                &mut matches,
            ),
            FileDiff::Modified { old, new } => {
                collect_matches(
                    old.canonical_text(),
                    &needle_lower,
                    SearchPane::Old,
                    &mut matches,
                );
                collect_matches(
                    new.canonical_text(),
                    &needle_lower,
                    SearchPane::New,
                    &mut matches,
                );
            }
        }
        matches
    }

    pub fn ensure_highlight(&mut self, theme: &Theme, selected: Option<&RepoPath>) {
        let Some(path) = selected else {
            return;
        };
        let Some(diff) = self
            .diffs()
            .iter()
            .find(|diff| diff.path() == path)
            .cloned()
        else {
            return;
        };
        let language = match &diff {
            FileDiff::Added { new } => new.language(),
            FileDiff::Deleted { old } => old.language(),
            FileDiff::Modified { old, .. } => old.language(),
        };
        let highlight_side =
            |file: &ProjectedFile| highlight::highlight(file.canonical_text(), language, theme);
        match &mut self.view {
            DiffViewState::Range(range) => {
                if range.highlight.contains(path) {
                    return;
                }
                let pair = diff_highlight(&diff, &highlight_side);
                range.highlight.insert(path.clone(), pair);
            }
            DiffViewState::Commits(commits) => {
                if commits.highlight.contains(path) {
                    return;
                }
                let pair = diff_highlight(&diff, &highlight_side);
                commits.highlight.insert(path.clone(), pair);
            }
        }
    }

    pub fn active_highlight(&self, selected: Option<&RepoPath>) -> Option<&DiffHighlight> {
        let path = selected?;
        match &self.view {
            DiffViewState::Range(range) => range.highlight.highlight(path),
            DiffViewState::Commits(commits) => commits.highlight.highlight(path),
        }
    }

    pub fn visual_row_for(&self, pane: SearchPane, line: usize) -> Option<usize> {
        self.rows().iter().position(|row| match pane {
            SearchPane::Old => row.old_number == Some(line),
            SearchPane::New => row.new_number == Some(line),
            SearchPane::Source => false,
        })
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn refresh_rows(&mut self, cx: &Cx) {
        let key = RowsKey {
            generation: self.generation,
            width: cx.size.width,
            height: cx.size.height,
            selected: cx.workspace.selected.clone(),
            tree_percent: cx.navigator_percent,
        };
        if self.rows.key.as_ref() == Some(&key) {
            return;
        }
        let rows = self.compute_rows(cx);
        self.rows = Arc::new(DiffRows {
            key: Some(key),
            rows,
        });
    }

    /// The wrapped rows for the active file at the current layout.
    pub fn compute_rows(&self, cx: &Cx) -> Vec<VisualRow> {
        let Some(active) = self.active_diff(cx.workspace.selected.as_ref()) else {
            return Vec::new();
        };
        let (old_width, new_width) = side_widths(cx, active);
        let empty = DiffHighlight::default();
        let highlights = self
            .active_highlight(cx.workspace.selected.as_ref())
            .unwrap_or(&empty);
        layout_diff(
            active,
            old_width,
            new_width,
            &highlights.old,
            &highlights.new,
            cx.theme,
        )
    }
}

fn diff_highlight(
    diff: &FileDiff,
    highlight_side: &impl Fn(&ProjectedFile) -> Vec<StyledLine>,
) -> DiffHighlight {
    match diff {
        FileDiff::Added { new } => DiffHighlight {
            old: Vec::new(),
            new: highlight_side(new),
        },
        FileDiff::Deleted { old } => DiffHighlight {
            old: highlight_side(old),
            new: Vec::new(),
        },
        FileDiff::Modified { old, new } => DiffHighlight {
            old: highlight_side(old),
            new: highlight_side(new),
        },
    }
}

pub fn diff_visible(diffs: &[FileDiff]) -> Vec<RepoPath> {
    diffs.iter().map(|diff| diff.path().clone()).collect()
}
