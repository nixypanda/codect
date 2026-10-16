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
//!
//! After a resize both body offsets are clamped and a hidden cursor row is
//! scrolled back into view; after a selection change a `show` body resets to the
//! top and re-anchors through the same rule.

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Paging {
    Ready,
    Loading,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSide {
    Show,
    Old,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub side: SearchSide,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Search {
    pub needle: String,
    pub matches: Vec<SearchMatch>,
    pub cursor: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug)]
pub enum Loaded {
    Show(Show),
    Diff(Diff),
}

impl Loaded {
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

    pub fn diff_rows(&self) -> &[VisualRow] {
        match self {
            Self::Show(_) => &[],
            Self::Diff(diff) => diff.rows(),
        }
    }

    pub(crate) fn focus(&self) -> Focus {
        match self {
            Self::Show(show) => match show.focus {
                ShowFocus::Tree => Focus::Tree,
                ShowFocus::Body => Focus::Content,
            },
            Self::Diff(diff) => diff.focus(),
        }
    }

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

    pub fn clamp_scroll(&mut self, height: u16) {
        match self {
            Self::Show(show) => {
                show.ensure_visible_rows();
                show.reanchor_cursor();
                show.clamp_view(height);
            }
            Self::Diff(diff) => {
                let max = diff.rows().len().saturating_sub(1) as u16;
                let current = diff.body_scroll();
                diff.set_body_scroll(current.min(max));
            }
        }
    }

    pub fn reset_body_scroll(&mut self) {
        match self {
            Self::Show(show) => {
                show.body = ShowBody {
                    scroll: 0,
                    hscroll: 0,
                    cursor_line: 1,
                };
                show.ensure_visible_rows();
            }
            Self::Diff(diff) => diff.set_body_scroll(0),
        }
    }

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

    pub fn palette_entries(&self) -> Vec<(Action, &'static str, &'static str)> {
        let mut entries = vec![
            (
                Action::Global(GlobalAction::Mode),
                "Switch types / signatures / tests",
                "m",
            ),
            (Action::Global(GlobalAction::Scope), "Change scope", "s"),
        ];
        match self {
            Self::Show(_) => {
                entries.push((Action::Show(ShowAction::EditRevision), "Edit revision", "r"));
                entries.extend([
                    (Action::Show(ShowAction::ToggleFold), "Toggle fold", "za"),
                    (Action::Show(ShowAction::OpenFold), "Open fold", "zo"),
                    (Action::Show(ShowAction::CloseFold), "Close fold", "zc"),
                    (
                        Action::Show(ShowAction::OpenRecursive),
                        "Open fold recursively",
                        "zO",
                    ),
                    (
                        Action::Show(ShowAction::CloseRecursive),
                        "Close fold recursively",
                        "zC",
                    ),
                    (Action::Show(ShowAction::OpenAll), "Open all folds", "zR"),
                    (Action::Show(ShowAction::CloseAll), "Close all folds", "zM"),
                ]);
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
                show.ensure_visible_rows();
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

#[derive(Clone, Copy, Debug)]
pub struct Ctx {
    pub height: u16,
}

#[derive(Debug)]
pub enum OutMsg {
    Effect(Cmd),
    Diagnose(String),
}

pub struct ViewCtx<'a> {
    pub theme: &'a Theme,
    pub busy: bool,
    pub tree_empty: bool,
}

pub(super) fn page_step(height: u16) -> u16 {
    (height.saturating_sub(3) / 2).max(1)
}

pub(super) fn scroll_by(action: GlobalAction, current: u16, step: u16, max: u16) -> u16 {
    match action {
        GlobalAction::Top => 0,
        GlobalAction::Bottom => max,
        GlobalAction::PageUp => current.saturating_sub(step),
        _ => current.saturating_add(step).min(max),
    }
}

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

pub(super) fn scroll_offset_to(index: usize, scroll: usize, height: usize) -> Option<u16> {
    if index < scroll {
        Some(index as u16)
    } else if height > 0 && index >= scroll + height {
        Some((index + 1 - height) as u16)
    } else {
        None
    }
}

pub(super) fn current_match(search: &Option<Search>) -> Option<SearchMatch> {
    let search = search.as_ref()?;
    search.matches.get(search.cursor).cloned()
}

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
