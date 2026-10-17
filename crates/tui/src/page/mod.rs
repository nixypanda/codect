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
use crate::plugin::{
    COMMITS_PANE, DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, FocusKey, NEW_PANE, OLD_PANE, SOURCE_FOCUS,
    SOURCE_PANE, TREE_FOCUS, TREE_PANE,
};
use crate::render::layout::{PaneSpec, Placement, VisualRow};
use crate::render::theme::Theme;
use crate::request::Request;
use crate::util::input::Key;

pub(crate) mod diff;
pub(crate) mod show;

pub(crate) use diff::{Commits, Diff, DiffSide, DiffViewState, Range};
pub(crate) use show::{Show, ShowBody};

const TREE_SPEC: PaneSpec = PaneSpec {
    id: TREE_PANE,
    placement: Placement::Navigator,
    focus: TREE_FOCUS,
};
const SOURCE_SPEC: PaneSpec = PaneSpec {
    id: SOURCE_PANE,
    placement: Placement::Body,
    focus: SOURCE_FOCUS,
};
const OLD_SPEC: PaneSpec = PaneSpec {
    id: OLD_PANE,
    placement: Placement::Body,
    focus: DIFF_BODY_FOCUS,
};
const NEW_SPEC: PaneSpec = PaneSpec {
    id: NEW_PANE,
    placement: Placement::Body,
    focus: DIFF_BODY_FOCUS,
};
const COMMITS_SPEC: PaneSpec = PaneSpec {
    id: COMMITS_PANE,
    placement: Placement::Band,
    focus: DIFF_BAND_FOCUS,
};

/// The tree and the two diff sides, plus the commits band when one is shown.
pub(crate) fn diff_panes(has_commits: bool) -> Vec<PaneSpec> {
    let mut panes = vec![TREE_SPEC, OLD_SPEC, NEW_SPEC];
    if has_commits {
        panes.push(COMMITS_SPEC);
    }
    panes
}

/// The focus ring: the distinct focus keys of `panes`, bands first, then
/// navigators, then bodies. Within a placement, registration order, then first
/// appearance. Panes that share a focus key (the diff's two sides) are one stop.
pub(crate) fn focus_ring(panes: &[PaneSpec]) -> Vec<FocusKey> {
    let mut ring: Vec<FocusKey> = Vec::new();
    for placement in [Placement::Band, Placement::Navigator, Placement::Body] {
        for spec in panes.iter().filter(|spec| spec.placement == placement) {
            if !ring.contains(&spec.focus) {
                ring.push(spec.focus);
            }
        }
    }
    ring
}

/// The default focus: the first navigator's focus key, else the first body's.
pub(crate) fn focus_default(panes: &[PaneSpec]) -> Option<FocusKey> {
    panes
        .iter()
        .find(|spec| spec.placement == Placement::Navigator)
        .or_else(|| panes.iter().find(|spec| spec.placement == Placement::Body))
        .map(|spec| spec.focus)
}

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

    /// The panes the loaded kind shows, in registration order: the tree first,
    /// then the view's body panes and its band.
    pub(crate) fn panes(&self) -> Vec<PaneSpec> {
        match self {
            Self::Show(_) => vec![TREE_SPEC, SOURCE_SPEC],
            Self::Diff(diff) => diff_panes(matches!(diff.view, DiffViewState::Commits(_))),
        }
    }

    /// The distinct focus keys this kind shows, navigator first. Unlike
    /// [`Self::panes`], this does not allocate, so the per-step focus
    /// normalisation stays off the hot path.
    pub(crate) fn focus_keys(&self) -> &'static [FocusKey] {
        static SHOW: [FocusKey; 2] = [TREE_FOCUS, SOURCE_FOCUS];
        static RANGE: [FocusKey; 2] = [TREE_FOCUS, DIFF_BODY_FOCUS];
        static COMMITS: [FocusKey; 3] = [TREE_FOCUS, DIFF_BODY_FOCUS, DIFF_BAND_FOCUS];
        match self {
            Self::Show(_) => &SHOW,
            Self::Diff(diff) => match diff.view {
                DiffViewState::Range(_) => &RANGE,
                DiffViewState::Commits(_) => &COMMITS,
            },
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
    /// The kernel's focus, so a page can tell its body pane from its band when
    /// both route through the same `update`.
    pub focus: Option<FocusKey>,
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
