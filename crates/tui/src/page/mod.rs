//! The page layer: the loaded projection and the state shared between its kinds.
//!
//! A session loads exactly one kind — `show` or `diff` — and no key switches
//! between them. Making the kind the top-level sum is therefore safe, and it
//! removes the states a flat model allowed: a commits cursor with a `show`, a
//! horizontal scroll on a diff, a base-revision prompt on a `show`, and so on.
//!
//! Each kind owns its model, its messages, and its `update` in its own module
//! ([`show`], [`diff`]). This module holds the discriminator and the vocabulary
//! both kinds share: scope, paging, search, and the tree's change badge, plus
//! the small pieces of runtime the shell folds back. The per-kind items are
//! re-exported here so a caller reads `page::Diff` rather than the submodule.

use base::{FileDiff, ProjectionMode, RepoPath};

use crate::action::{Action, CommitsAction, GlobalAction, RangeAction, ShowAction};
use crate::api::Cmd;
use crate::component::text_input::Edit;
use crate::render::layout::VisualRow;
use crate::render::metrics::Focus;
use crate::render::theme::Theme;
use crate::util::input::Key;

pub(crate) mod diff;
pub(crate) mod show;

pub(crate) use diff::{Commits, CommitsFocus, Diff, DiffSide, DiffViewState, Range};
pub(crate) use show::{Show, ShowBody, ShowFocus};

/// The projection modes the frontend can select.
pub(crate) fn available_modes() -> [ProjectionMode; 3] {
    [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ]
}

pub(crate) fn mode_label(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
        ProjectionMode::Tests => "tests",
    }
}

/// A projection scope: the selection and its short status-bar label.
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

/// Whether a projection is being loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Paging {
    Ready,
    Loading,
}

/// Which projection a search match belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSide {
    Show,
    Old,
    New,
}

/// One occurrence of the search needle on a line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub side: SearchSide,
    /// One-based line number.
    pub line: usize,
    /// Byte range within the line.
    pub start: usize,
    pub end: usize,
}

/// A committed search: highlights persist after the input closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Search {
    pub needle: String,
    pub matches: Vec<SearchMatch>,
    pub cursor: usize,
}

/// How a file changed in a focused diff, for tree badges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

// ---------------------------------------------------------------------------
// The loaded sum
// ---------------------------------------------------------------------------

/// The projection currently loaded.
#[derive(Clone, Debug)]
pub enum Loaded {
    Show(Show),
    Diff(Diff),
}

impl Loaded {
    /// The tree paths the current projection shows.
    pub fn visible(&self) -> &[RepoPath] {
        match self {
            Self::Show(show) => &show.visible,
            Self::Diff(diff) => &diff.visible,
        }
    }

    pub fn selected(&self) -> Option<&RepoPath> {
        match self {
            Self::Show(show) => show.selected.as_ref(),
            Self::Diff(diff) => diff.selected.as_ref(),
        }
    }

    pub fn set_selected(&mut self, selected: Option<RepoPath>) {
        match self {
            Self::Show(show) => show.selected = selected,
            Self::Diff(diff) => diff.selected = selected,
        }
    }

    pub fn search(&self) -> Option<&Search> {
        match self {
            Self::Show(show) => show.search.as_ref(),
            Self::Diff(diff) => diff.search.as_ref(),
        }
    }

    pub fn mode(&self) -> ProjectionMode {
        match self {
            Self::Show(show) => show.mode,
            Self::Diff(diff) => diff.mode,
        }
    }

    pub fn scope(&self) -> &Scope {
        match self {
            Self::Show(show) => &show.scope,
            Self::Diff(diff) => &diff.scope,
        }
    }

    pub fn paging(&self) -> Paging {
        match self {
            Self::Show(show) => show.paging,
            Self::Diff(diff) => diff.paging,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.paging() == Paging::Loading
    }

    /// The wrapped visual rows of the active diff; empty for a `show`.
    pub fn diff_rows(&self) -> &[VisualRow] {
        match self {
            Self::Show(_) => &[],
            Self::Diff(diff) => diff.rows(),
        }
    }

    /// The focus, normalized for layout.
    pub(crate) fn focus(&self) -> Focus {
        match self {
            Self::Show(show) => match show.focus {
                ShowFocus::Tree => Focus::Tree,
                ShowFocus::Body => Focus::Content,
            },
            Self::Diff(diff) => diff.focus(),
        }
    }

    /// Moves focus to the pane a hit test selected.
    pub(crate) fn set_focus(&mut self, focus: Focus) {
        match self {
            Self::Show(show) => {
                show.focus = match focus {
                    Focus::Tree => ShowFocus::Tree,
                    _ => ShowFocus::Body,
                };
            }
            Self::Diff(diff) => diff.set_focus(focus),
        }
    }

    pub fn prompt_active(&self) -> bool {
        match self {
            Self::Show(show) => show.prompt.is_some(),
            Self::Diff(diff) => diff.prompt.is_some(),
        }
    }

    /// The largest vertical scroll offset the body can take.
    pub fn max_scroll(&self) -> u16 {
        let count = match self {
            Self::Show(show) => show.line_count(),
            Self::Diff(diff) => diff.rows().len(),
        };
        count.saturating_sub(1) as u16
    }

    /// The largest horizontal scroll offset the body can take.
    pub fn max_hscroll(&self) -> u16 {
        match self {
            Self::Show(show) => show.max_line_width().saturating_sub(1) as u16,
            Self::Diff(_) => 0,
        }
    }

    /// Clamps both scroll offsets after the terminal changed size.
    pub fn clamp_scroll(&mut self) {
        let max = self.max_scroll();
        let max_h = self.max_hscroll();
        match self {
            Self::Show(show) => {
                show.body.scroll = show.body.scroll.min(max);
                show.body.hscroll = show.body.hscroll.min(max_h);
            }
            Self::Diff(diff) => {
                let current = diff.body_scroll();
                diff.set_body_scroll(current.min(max));
            }
        }
    }

    /// Resets the body to the top after the selection changed.
    pub fn reset_body_scroll(&mut self) {
        match self {
            Self::Show(show) => {
                show.body = ShowBody {
                    scroll: 0,
                    hscroll: 0,
                }
            }
            Self::Diff(diff) => diff.set_body_scroll(0),
        }
    }

    /// The tree badge for a path, when a diff marks it.
    pub fn change_kind(&self, path: &RepoPath) -> Option<ChangeKind> {
        let Self::Diff(diff) = self else {
            return None;
        };
        let file = diff.diffs().iter().find(|file| file.path() == path)?;
        Some(match file {
            FileDiff::Added { .. } => ChangeKind::Added,
            FileDiff::Deleted { .. } => ChangeKind::Deleted,
            FileDiff::Modified { .. } => ChangeKind::Modified,
        })
    }

    /// The actions the command palette offers for this projection.
    pub fn palette_entries(&self) -> Vec<(Action, &'static str, &'static str)> {
        let mut entries = vec![
            (
                Action::Global(GlobalAction::Mode),
                "Switch Types / Signatures",
                "m",
            ),
            (Action::Global(GlobalAction::Scope), "Change scope", "s"),
        ];
        match self {
            Self::Show(_) => {
                entries.push((Action::Show(ShowAction::EditRevision), "Edit revision", "r"))
            }
            Self::Diff(diff) => match &diff.view {
                DiffViewState::Range(_) => {
                    entries.push((
                        Action::Range(RangeAction::EditBase),
                        "Edit base revision",
                        "b",
                    ));
                    entries.push((
                        Action::Range(RangeAction::EditTarget),
                        "Edit target revision",
                        "t",
                    ));
                    // A snapshot endpoint has no first-parent history to walk,
                    // and a merge-base base need not lie on the target's
                    // first-parent chain, so the commits view is not offered.
                    if !engine::is_snapshot_spec(&diff.base)
                        && !engine::is_snapshot_spec(&diff.target)
                        && !diff.merge_base
                    {
                        entries.push((
                            Action::Range(RangeAction::SwitchToCommits),
                            "Switch to commits view",
                            "",
                        ));
                    }
                }
                DiffViewState::Commits(_) => entries.push((
                    Action::Commits(CommitsAction::SwitchToRange),
                    "Switch to range view",
                    "",
                )),
            },
        }
        entries.extend([
            (Action::Global(GlobalAction::Finder), "Find file", "Ctrl-F"),
            (Action::Global(GlobalAction::Search), "Search in view", "/"),
            (Action::Global(GlobalAction::NextPane), "Switch pane", "Tab"),
            (
                Action::Global(GlobalAction::TreeWider),
                "Widen file tree",
                "]",
            ),
            (
                Action::Global(GlobalAction::TreeNarrower),
                "Narrow file tree",
                "[",
            ),
            (
                Action::Global(GlobalAction::TreeReset),
                "Reset file tree",
                "\\",
            ),
            (Action::Global(GlobalAction::Top), "Jump to top", "g"),
            (Action::Global(GlobalAction::Bottom), "Jump to bottom", "G"),
            (Action::Global(GlobalAction::Help), "Help", "?"),
            (Action::Global(GlobalAction::Quit), "Quit", "q"),
        ]);
        entries
    }

    /// Runs the selection-dependent work a frame needs: highlighting, the
    /// committed-search re-sync, and the wrapped diff rows.
    pub fn settle(
        &mut self,
        theme: &Theme,
        visible_body: bool,
        width: u16,
        height: u16,
        tree_percent: u16,
    ) {
        match self {
            Self::Show(show) => {
                if visible_body {
                    show.ensure_highlight(theme);
                }
                show.resync_search();
            }
            Self::Diff(diff) => {
                if visible_body {
                    diff.ensure_highlight(theme);
                }
                diff.resync_search();
                diff.refresh_rows(width, height, tree_percent, theme);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Page runtime
// ---------------------------------------------------------------------------

/// The read-only context a page `update` reads from the shell: the terminal
/// height, which paging arithmetic needs. Everything else a page acts on is its
/// own state.
#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    pub height: u16,
}

/// What a page asks the shell to do after handling a message. A page never
/// mutates the tree, chrome, overlay, or diagnostic itself.
#[derive(Debug)]
pub enum OutMsg {
    /// Describe an effect; the shell folds it into the runtime's command list.
    Effect(Cmd),
    /// Show a self-expiring diagnostic.
    Diagnose(String),
}

/// The read-only context a page `view` reads: the design tokens and the shell
/// flags that decide an empty or loading pane.
pub struct ViewCtx<'a> {
    pub theme: &'a Theme,
    pub busy: bool,
    pub tree_empty: bool,
}

/// How many rows one page key moves.
pub(super) fn page_step(height: u16) -> u16 {
    (height.saturating_sub(3) / 2).max(1)
}

/// The scroll offset a top/bottom/page key selects.
pub(super) fn scroll_by(action: GlobalAction, current: u16, step: u16, max: u16) -> u16 {
    match action {
        GlobalAction::Top => 0,
        GlobalAction::Bottom => max,
        GlobalAction::PageUp => current.saturating_sub(step),
        _ => current.saturating_add(step).min(max),
    }
}

/// Advances a committed search and returns the newly current match.
pub(super) fn advance_search(search: &mut Option<Search>, forward: bool) -> Option<SearchMatch> {
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

/// The scroll offset that brings a match at `index` into a viewport of
/// `height`, or `None` when it is already visible.
pub(super) fn scroll_offset_to(index: usize, scroll: usize, height: usize) -> Option<u16> {
    if index < scroll {
        Some(index as u16)
    } else if height > 0 && index >= scroll + height {
        Some((index + 1 - height) as u16)
    } else {
        None
    }
}

/// The match the committed search is currently on.
pub(super) fn current_match(search: &Option<Search>) -> Option<SearchMatch> {
    let search = search.as_ref()?;
    search.matches.get(search.cursor).cloned()
}

/// The text edit a key performs while a prompt is open.
pub(super) fn text_edit(key: Key) -> Option<Edit> {
    match key {
        Key::Char(character) => Some(Edit::Insert(character)),
        Key::Backspace => Some(Edit::Backspace),
        Key::Delete => Some(Edit::Delete),
        Key::Left => Some(Edit::Left),
        Key::Right => Some(Edit::Right),
        Key::Home => Some(Edit::Home),
        Key::End => Some(Edit::End),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Search helpers
// ---------------------------------------------------------------------------

fn search_ranges(
    search: Option<&Search>,
    side: SearchSide,
    line: usize,
) -> Vec<(usize, usize, bool)> {
    let Some(search) = search else {
        return Vec::new();
    };
    search
        .matches
        .iter()
        .enumerate()
        .filter(|(_, matched)| matched.side == side && matched.line == line)
        .map(|(index, matched)| (matched.start, matched.end, index == search.cursor))
        .collect()
}

fn collect_matches(text: &str, needle_lower: &str, side: SearchSide, out: &mut Vec<SearchMatch>) {
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
                    side,
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
