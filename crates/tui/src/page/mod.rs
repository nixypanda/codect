//! The page layer: the loaded projection and the state shared between its kinds.
//!
//! A session loads exactly one kind — `show` or `diff` — and no key switches
//! between them. Making the kind the top-level sum is therefore safe, and it
//! removes the states a flat model allowed: a commits cursor with a `show`, a
//! horizontal scroll on a diff, a base-revision prompt on a `show`, and so on.
//!
//! Each kind owns its model, its messages, and its `update` in its own module
//! ([`show`], [`diff`]). This module holds the discriminator and the vocabulary
//! both kinds share: scope, paging, and the tree's change badge, plus the small
//! pieces of runtime the shell folds back. The per-kind items are re-exported
//! here so a caller reads `page::Diff` rather than the submodule.
//!
//! After a resize both body offsets are clamped and a hidden cursor row is
//! scrolled back into view; after a selection change a `show` body resets to the
//! top and re-anchors through the same rule.

use base::{ProjectionMode, RepoPath};

use crate::action::{Action, GlobalAction, PrefixGroup};
use crate::api::Cmd;
use crate::component::text_input::Edit;
use crate::render::layout::VisualRow;
use crate::render::metrics::Focus;
use crate::render::theme::Theme;
use crate::request::Request;
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

#[derive(Clone, Debug)]
pub enum Loaded {
    Show(Show),
    Diff(Diff),
}

impl Loaded {
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

    pub fn clamp_scroll(&mut self, height: u16, selected: Option<&RepoPath>) {
        match self {
            Self::Show(show) => {
                show.ensure_visible_rows(selected);
                show.reanchor_cursor(selected);
                show.clamp_view(height, selected);
            }
            Self::Diff(diff) => {
                let max = diff.rows().len().saturating_sub(1) as u16;
                let current = diff.body_scroll();
                diff.set_body_scroll(current.min(max));
            }
        }
    }

    pub fn reset_body_scroll(&mut self, selected: Option<&RepoPath>) {
        match self {
            Self::Show(show) => {
                show.body = ShowBody {
                    scroll: 0,
                    hscroll: 0,
                    cursor_line: 1,
                };
                show.ensure_visible_rows(selected);
            }
            Self::Diff(diff) => diff.set_body_scroll(0),
        }
    }

    pub fn palette_entries(&self) -> Vec<(Action, &'static str, &'static str)> {
        crate::command::palette_entries(self)
    }

    pub fn prefix_groups(&self) -> Vec<PrefixGroup> {
        crate::command::prefix_groups(self)
    }

    /// The rows, folds, and highlighting a page owns, run before the shell
    /// re-syncs the search.
    pub fn settle_rows(&mut self, theme: &Theme, visible_body: bool, selected: Option<&RepoPath>) {
        match self {
            Self::Show(show) => {
                show.ensure_visible_rows(selected);
                if visible_body {
                    show.ensure_highlight(theme, selected);
                }
            }
            Self::Diff(diff) => {
                if visible_body {
                    diff.ensure_highlight(theme, selected);
                }
            }
        }
    }

    /// The diff row layout, run after the shell re-syncs the search. A show page
    /// keeps its row cache in the page itself.
    pub fn refresh_rows(
        &mut self,
        width: u16,
        height: u16,
        tree_percent: u16,
        theme: &Theme,
        selected: Option<&RepoPath>,
    ) {
        if let Self::Diff(diff) = self {
            diff.refresh_rows(width, height, tree_percent, theme, selected);
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
    Request(Request),
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

pub(super) fn scroll_offset_to(index: usize, scroll: usize, height: usize) -> Option<u16> {
    if index < scroll {
        Some(index as u16)
    } else if height > 0 && index >= scroll + height {
        Some((index + 1 - height) as u16)
    } else {
        None
    }
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
