//! The `diff` page: a base/target range compared directly, or the first-parent
//! steps of a commits view, each with its own scroll, search, and highlighting.

use std::sync::Arc;

use base::{DiffRowKind, FileDiff, ProjectedFile, ProjectionMode, RepoPath, Selection};
use engine::EngineError;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::action::GlobalAction;
use crate::api::{Cmd, HistoryPayload};
use crate::component::commit_picker::{self, CommitPicker};
use crate::component::text_input::TextInput;
use crate::render::block::pane_block;
use crate::render::empty::render_empty;
use crate::render::highlight::{self, Run, StyledLine};
use crate::render::layout::{
    Edge, VisualRow, VisualRowKind, commit_offset, frame_areas, gutter_width, layout_diff,
    split_with_dividers,
};
use crate::render::metrics::{Focus, SIDE_BY_SIDE_MIN_WIDTH};
use crate::render::text::truncate_ellipsis;
use crate::render::theme::Theme;
use crate::route::{DiffRequest, DiffView};
use crate::util::cache::BoundedCache;
use crate::util::input::Key;

use super::{
    Ctx, OutMsg, Paging, Scope, Search, SearchMatch, SearchSide, ViewCtx, advance_search,
    collect_matches, current_match, page_step, scroll_by, scroll_offset_to, search_ranges,
    text_edit,
};

/// Which side of a diff a prompt edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffSide {
    Base,
    Target,
}

/// The base- or target-revision editor, which only exists on a diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPrompt {
    pub side: DiffSide,
    pub input: TextInput,
}

/// The panes `Tab` cycles in a diff range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeFocus {
    Tree,
    Diff,
}

/// The panes `Tab` cycles in a diff's commits view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitsFocus {
    Commits,
    Tree,
    Diff,
}

/// The body scroll offset of a diff. Diff panes always wrap, so there is no
/// horizontal offset to represent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffBody {
    pub scroll: u16,
}

/// Syntax-highlighted lines for the selected diff, one side at a time.
#[derive(Clone, Default)]
pub struct DiffHighlight {
    pub old: Vec<StyledLine>,
    pub new: Vec<StyledLine>,
}

/// Bounded, per-path syntax highlighting for a diff.
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

    pub fn insert(&mut self, path: RepoPath, highlight: DiffHighlight) {
        self.cache.insert(path, Arc::new(highlight));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// A loaded `diff` projection.
#[derive(Clone, Debug)]
pub struct Diff {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub scope: Scope,
    pub visible: Arc<[RepoPath]>,
    pub selected: Option<RepoPath>,
    pub search: Option<Search>,
    pub paging: Paging,
    /// The base- or target-revision editor, which only exists on a diff.
    pub prompt: Option<DiffPrompt>,
    pub view: DiffViewState,
    /// Bumped whenever the diffs change, invalidating [`Self::rows`].
    generation: u64,
    /// The wrapped visual rows of the active diff, cached so a frame or a
    /// scroll key does not recompute them.
    rows: Arc<DiffRows>,
}

/// The inputs a cached diff layout depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RowsKey {
    generation: u64,
    width: u16,
    height: u16,
    selected: Option<RepoPath>,
    tree_percent: u16,
}

#[derive(Clone, Debug, Default)]
struct DiffRows {
    key: Option<RowsKey>,
    rows: Vec<VisualRow>,
}

/// A diff's two views: the endpoint range, or the first-parent steps.
#[derive(Clone, Debug)]
pub enum DiffViewState {
    Range(Range),
    Commits(Commits),
}

/// The endpoint range: `base..target`, compared directly.
#[derive(Clone, Debug)]
pub struct Range {
    pub focus: RangeFocus,
    pub body: DiffBody,
    pub diffs: Arc<[FileDiff]>,
    pub highlight: DiffHighlights,
}

/// The commits view: a picker above the file tree, one step's diff shown.
#[derive(Clone, Debug)]
pub struct Commits {
    pub focus: CommitsFocus,
    pub picker: CommitPicker,
    pub body: DiffBody,
    pub diffs: Arc<[FileDiff]>,
    pub highlight: DiffHighlights,
}

impl Range {
    pub fn empty() -> Self {
        Self {
            focus: RangeFocus::Tree,
            body: DiffBody { scroll: 0 },
            diffs: Arc::from(Vec::new()),
            highlight: DiffHighlights::default(),
        }
    }
}

impl Commits {
    pub fn empty() -> Self {
        Self {
            focus: CommitsFocus::Tree,
            picker: CommitPicker::empty(),
            body: DiffBody { scroll: 0 },
            diffs: Arc::from(Vec::new()),
            highlight: DiffHighlights::default(),
        }
    }
}

impl Diff {
    /// A diff with no diffs yet, still loading.
    pub fn new(
        base: String,
        target: String,
        mode: ProjectionMode,
        scope: Scope,
        view: DiffViewState,
    ) -> Self {
        Self {
            base,
            target,
            mode,
            scope,
            visible: Arc::from(Vec::new()),
            selected: None,
            search: None,
            paging: Paging::Loading,
            prompt: None,
            view,
            generation: 0,
            rows: Arc::new(DiffRows::default()),
        }
    }

    /// Marks the diffs changed, so [`Self::refresh_rows`] recomputes.
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// The wrapped visual rows of the active diff.
    pub fn rows(&self) -> &[VisualRow] {
        self.rows.rows.as_slice()
    }

    /// Recomputes the wrapped rows when the layout inputs changed.
    pub fn refresh_rows(&mut self, width: u16, height: u16, tree_percent: u16, theme: &Theme) {
        let key = RowsKey {
            generation: self.generation,
            width,
            height,
            selected: self.selected.clone(),
            tree_percent,
        };
        if self.rows.key.as_ref() == Some(&key) {
            return;
        }
        let rows = self.compute_rows(width, height, tree_percent, theme);
        self.rows = Arc::new(DiffRows {
            key: Some(key),
            rows,
        });
    }

    /// The wrapped rows recomputed without the cache.
    pub fn compute_rows(
        &self,
        width: u16,
        height: u16,
        tree_percent: u16,
        theme: &Theme,
    ) -> Vec<VisualRow> {
        let Some(active) = self.active_diff() else {
            return Vec::new();
        };
        let (_, content, _) = frame_areas(width, height);
        let (old_width, new_width) = self.side_widths(width, content, tree_percent, active, theme);
        let empty = DiffHighlight::default();
        let highlights = self.active_highlight().unwrap_or(&empty);
        layout_diff(
            active,
            old_width,
            new_width,
            &highlights.old,
            &highlights.new,
            theme,
        )
    }

    fn side_widths(
        &self,
        width: u16,
        content: Rect,
        tree_percent: u16,
        diff: &FileDiff,
        theme: &Theme,
    ) -> (usize, usize) {
        let gutter = gutter_width(diff);
        if width >= SIDE_BY_SIDE_MIN_WIDTH {
            let rest = 100 - tree_percent;
            let side = rest / 2;
            let (columns, _) = split_with_dividers(content, &[tree_percent, side, rest - side]);
            let old_inner = pane_block("", false, theme, Edge::Middle).inner(columns[1]);
            let new_inner = pane_block("", false, theme, Edge::Right).inner(columns[2]);
            (
                (old_inner.width as usize).saturating_sub(gutter),
                (new_inner.width as usize).saturating_sub(gutter),
            )
        } else {
            let inner = pane_block("", false, theme, Edge::Solo).inner(content);
            let width = (inner.width as usize).saturating_sub(gutter);
            (width, width)
        }
    }

    /// The visual row index of a search match, if the diff is laid out.
    pub fn visual_row_for(&self, side: SearchSide, line: usize) -> Option<usize> {
        self.rows().iter().position(|row| match side {
            SearchSide::Old => row.old_number == Some(line),
            SearchSide::New => row.new_number == Some(line),
            SearchSide::Show => false,
        })
    }

    /// The focus, normalized for layout.
    pub(crate) fn focus(&self) -> Focus {
        match &self.view {
            DiffViewState::Range(range) => match range.focus {
                RangeFocus::Tree => Focus::Tree,
                RangeFocus::Diff => Focus::Content,
            },
            DiffViewState::Commits(commits) => match commits.focus {
                CommitsFocus::Commits => Focus::Commits,
                CommitsFocus::Tree => Focus::Tree,
                CommitsFocus::Diff => Focus::Content,
            },
        }
    }

    /// Moves focus to the pane a hit test selected.
    pub(crate) fn set_focus(&mut self, focus: Focus) {
        match &mut self.view {
            DiffViewState::Range(range) => {
                range.focus = match focus {
                    Focus::Tree => RangeFocus::Tree,
                    _ => RangeFocus::Diff,
                };
            }
            DiffViewState::Commits(commits) => {
                commits.focus = match focus {
                    Focus::Commits => CommitsFocus::Commits,
                    Focus::Tree => CommitsFocus::Tree,
                    Focus::Content => CommitsFocus::Diff,
                };
            }
        }
    }
}

impl Diff {
    /// The diffs of the current view.
    pub fn diffs(&self) -> &[FileDiff] {
        match &self.view {
            DiffViewState::Range(range) => &range.diffs,
            DiffViewState::Commits(commits) => &commits.diffs,
        }
    }

    /// The diff currently shown, if a file is selected.
    pub fn active_diff(&self) -> Option<&FileDiff> {
        let path = self.selected.as_ref()?;
        self.diffs().iter().find(|diff| diff.path() == path)
    }

    pub fn focus_is_diff(&self) -> bool {
        match &self.view {
            DiffViewState::Range(range) => range.focus == RangeFocus::Diff,
            DiffViewState::Commits(commits) => commits.focus == CommitsFocus::Diff,
        }
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

    /// The revisions the visible diff compares, as short labels.
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

    /// The search ranges on one line of one diff side.
    pub fn search_ranges(&self, side: SearchSide, line: usize) -> Vec<(usize, usize, bool)> {
        search_ranges(self.search.as_ref(), side, line)
    }

    fn search_matches(&self, needle: &str) -> Vec<SearchMatch> {
        let needle_lower = needle.to_lowercase();
        let mut matches = Vec::new();
        let Some(diff) = self.active_diff() else {
            return matches;
        };
        match diff {
            FileDiff::Added { new } => collect_matches(
                new.canonical_text(),
                &needle_lower,
                SearchSide::New,
                &mut matches,
            ),
            FileDiff::Deleted { old } => collect_matches(
                old.canonical_text(),
                &needle_lower,
                SearchSide::Old,
                &mut matches,
            ),
            FileDiff::Modified { old, new } => {
                collect_matches(
                    old.canonical_text(),
                    &needle_lower,
                    SearchSide::Old,
                    &mut matches,
                );
                collect_matches(
                    new.canonical_text(),
                    &needle_lower,
                    SearchSide::New,
                    &mut matches,
                );
            }
        }
        matches
    }

    /// Recomputes the committed search for the current selection.
    pub fn set_search(&mut self, needle: &str) {
        if needle.is_empty() {
            self.search = None;
            return;
        }
        self.search = Some(Search {
            needle: needle.to_owned(),
            matches: self.search_matches(needle),
            cursor: 0,
        });
    }

    /// Recomputes a committed search after the selection changed.
    pub fn resync_search(&mut self) {
        let Some(needle) = self.search.as_ref().map(|search| search.needle.clone()) else {
            return;
        };
        let matches = self.search_matches(&needle);
        if let Some(search) = &mut self.search {
            search.matches = matches;
            search.cursor = search.cursor.min(search.matches.len().saturating_sub(1));
        }
    }

    /// Computes and caches highlighting for the selected diff when missing.
    pub fn ensure_highlight(&mut self, theme: &Theme) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        let Some(diff) = self
            .diffs()
            .iter()
            .find(|diff| *diff.path() == path)
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
                if range.highlight.contains(&path) {
                    return;
                }
                let pair = diff_highlight(&diff, &highlight_side);
                range.highlight.insert(path, pair);
            }
            DiffViewState::Commits(commits) => {
                if commits.highlight.contains(&path) {
                    return;
                }
                let pair = diff_highlight(&diff, &highlight_side);
                commits.highlight.insert(path, pair);
            }
        }
    }

    /// The current view's highlight for the selected diff, if computed.
    pub fn active_highlight(&self) -> Option<&DiffHighlight> {
        let path = self.selected.as_ref()?;
        match &self.view {
            DiffViewState::Range(range) => range.highlight.highlight(path),
            DiffViewState::Commits(commits) => commits.highlight.highlight(path),
        }
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

/// Builds a diff scope's visible paths from its diffs.
pub fn diff_visible(diffs: &[FileDiff]) -> Vec<RepoPath> {
    diffs.iter().map(|diff| diff.path().clone()).collect()
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// A message the `diff` page understands.
#[derive(Debug)]
pub enum Msg {
    /// A key that is not a semantic action, routed while a diff pane has focus.
    Key(Key),
    /// A wheel scroll of the diff body by a signed number of rows.
    Scroll(i32),
    /// A scroll action: top, bottom, page up, or page down.
    Action(GlobalAction),
    /// Advance the committed search to the next or previous match.
    StepSearch(bool),
    /// Cycle the focused pane forward or backward.
    CycleFocus(bool),
    /// A key typed while a revision prompt is open.
    Prompt(Key),
    /// Open the prompt for one side of the range.
    EditSide(DiffSide),
    /// Reload the projection in another view.
    SwitchView(DiffView),
    /// Replace the committed search.
    SetSearch(String),
    /// Drop the committed search.
    ClearSearch,
    /// Scroll the current search match into view without advancing.
    ScrollToCurrent,
    /// A commit picker message.
    Commit(commit_picker::Msg),
    /// Reload the same projection with a new scope.
    ReloadScope(Selection),
    /// Reload the same projection in another mode.
    SwitchMode(ProjectionMode),
    /// A `diff` range effect finished.
    Loaded(DiffRequest, Result<Arc<[FileDiff]>, Box<EngineError>>),
    /// A commits-view effect finished: the steps and the first step's diff.
    HistoryLoaded(DiffRequest, Result<HistoryPayload, Box<EngineError>>),
    /// A single commit step's projection finished.
    StepLoaded {
        index: usize,
        inner_height: usize,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
}

/// The pure `diff` update. It mutates only the page and returns the shell work
/// it produced.
pub fn update(msg: Msg, page: &mut Diff, ctx: &Ctx) -> Vec<OutMsg> {
    match msg {
        Msg::Key(key) => body_key(page, key),
        Msg::Scroll(delta) => {
            scroll_body(page, delta);
            Vec::new()
        }
        Msg::Action(action) => {
            action_apply(page, action, ctx);
            Vec::new()
        }
        Msg::StepSearch(forward) => {
            step_search(page, forward, ctx);
            Vec::new()
        }
        Msg::CycleFocus(forward) => {
            cycle_focus(page, forward);
            Vec::new()
        }
        Msg::Prompt(key) => prompt_key(page, key),
        Msg::EditSide(side) => {
            let value = match side {
                DiffSide::Base => page.base.clone(),
                DiffSide::Target => page.target.clone(),
            };
            page.prompt = Some(DiffPrompt {
                side,
                input: TextInput::new(value),
            });
            Vec::new()
        }
        Msg::SwitchView(view) => switch_view(page, view),
        Msg::SetSearch(needle) => {
            page.set_search(&needle);
            Vec::new()
        }
        Msg::ClearSearch => {
            page.search = None;
            Vec::new()
        }
        Msg::ScrollToCurrent => {
            if let Some(matched) = current_match(&page.search) {
                let index = page.visual_row_for(matched.side, matched.line).unwrap_or(0);
                if let Some(next) = scroll_offset_to(
                    index,
                    page.body_scroll() as usize,
                    ctx.height.saturating_sub(4) as usize,
                ) {
                    page.set_body_scroll(next);
                }
            }
            Vec::new()
        }
        Msg::Commit(msg) => {
            if let DiffViewState::Commits(commits) = &mut page.view {
                commits.picker.update(msg);
            }
            Vec::new()
        }
        Msg::ReloadScope(selection) => {
            page.paging = Paging::Loading;
            let mut request = page.request();
            request.selection = selection;
            vec![OutMsg::Effect(Cmd::Diff(request))]
        }
        Msg::SwitchMode(mode) => {
            page.paging = Paging::Loading;
            let mut request = page.request();
            request.mode = mode;
            vec![OutMsg::Effect(Cmd::Diff(request))]
        }
        Msg::Loaded(request, result) => loaded(page, request, result),
        Msg::HistoryLoaded(request, result) => history_loaded(page, request, result),
        Msg::StepLoaded {
            index,
            inner_height,
            result,
        } => step_loaded(page, index, inner_height, result),
    }
}

impl Diff {
    /// The effect request that reloads the projection exactly as it is.
    pub fn request(&self) -> DiffRequest {
        DiffRequest {
            base: self.base.clone(),
            target: self.target.clone(),
            mode: self.mode,
            selection: self.scope.selection.clone(),
            view: match &self.view {
                DiffViewState::Range(_) => DiffView::Range,
                DiffViewState::Commits(_) => DiffView::Commits,
            },
        }
    }
}

fn body_key(page: &mut Diff, key: Key) -> Vec<OutMsg> {
    match page.focus() {
        Focus::Commits => {
            if let Some(msg) = commit_key(key)
                && let DiffViewState::Commits(commits) = &mut page.view
            {
                commits.picker.update(msg);
            }
        }
        Focus::Content => {
            let max = page.rows().len().saturating_sub(1) as u16;
            let scroll = match key {
                Key::Down | Key::Char('j') => page.body_scroll().saturating_add(1).min(max),
                Key::Up | Key::Char('k') => page.body_scroll().saturating_sub(1),
                _ => return Vec::new(),
            };
            page.set_body_scroll(scroll);
        }
        Focus::Tree => {}
    }
    Vec::new()
}

fn scroll_body(page: &mut Diff, delta: i32) {
    let max = page.rows().len().saturating_sub(1) as u16;
    let scroll = page.body_scroll();
    let next = if delta < 0 {
        scroll.saturating_sub(delta.unsigned_abs() as u16)
    } else {
        scroll.saturating_add(delta as u16).min(max)
    };
    page.set_body_scroll(next);
}

fn action_apply(page: &mut Diff, action: GlobalAction, ctx: &Ctx) {
    let step = page_step(ctx.height);
    match page.focus() {
        Focus::Commits => {
            if let DiffViewState::Commits(commits) = &mut page.view {
                commits.picker.update(commit_move_msg(action, step));
            }
        }
        Focus::Content => {
            let max = page.rows().len().saturating_sub(1) as u16;
            let scroll = scroll_by(action, page.body_scroll(), step, max);
            page.set_body_scroll(scroll);
        }
        Focus::Tree => {}
    }
}

fn step_search(page: &mut Diff, forward: bool, ctx: &Ctx) {
    if let Some(matched) = advance_search(&mut page.search, forward) {
        let index = page.visual_row_for(matched.side, matched.line).unwrap_or(0);
        if let Some(next) = scroll_offset_to(
            index,
            page.body_scroll() as usize,
            ctx.height.saturating_sub(4) as usize,
        ) {
            page.set_body_scroll(next);
        }
    }
}

fn cycle_focus(page: &mut Diff, forward: bool) {
    match &mut page.view {
        DiffViewState::Range(range) => {
            range.focus = match (range.focus, forward) {
                (RangeFocus::Tree, true) => RangeFocus::Diff,
                (RangeFocus::Tree, false) => RangeFocus::Diff,
                (RangeFocus::Diff, _) => RangeFocus::Tree,
            };
        }
        DiffViewState::Commits(commits) => {
            let order = [
                CommitsFocus::Commits,
                CommitsFocus::Tree,
                CommitsFocus::Diff,
            ];
            let index = order
                .iter()
                .position(|focus| *focus == commits.focus)
                .unwrap_or(0);
            let next = if forward {
                (index + 1) % order.len()
            } else {
                (index + order.len() - 1) % order.len()
            };
            commits.focus = order[next];
        }
    }
}

fn switch_view(page: &mut Diff, view: DiffView) -> Vec<OutMsg> {
    let matches = matches!(
        (&page.view, view),
        (DiffViewState::Range(_), DiffView::Commits) | (DiffViewState::Commits(_), DiffView::Range)
    );
    if !matches {
        return Vec::new();
    }
    page.paging = Paging::Loading;
    let mut request = page.request();
    request.view = view;
    vec![OutMsg::Effect(Cmd::Diff(request))]
}

fn prompt_key(page: &mut Diff, key: Key) -> Vec<OutMsg> {
    let Some(mut prompt) = page.prompt.take() else {
        return Vec::new();
    };
    let mut reopen = true;
    let mut effect = None;
    match key {
        Key::Esc => reopen = false,
        Key::Enter => {
            reopen = false;
            let value = prompt.input.value();
            if !value.is_empty() {
                let mut request = page.request();
                match prompt.side {
                    DiffSide::Base => request.base = value,
                    DiffSide::Target => request.target = value,
                }
                page.paging = Paging::Loading;
                effect = Some(Cmd::Diff(request));
            }
        }
        key => {
            if let Some(edit) = text_edit(key) {
                prompt.input.edit(edit);
            }
        }
    }
    if reopen {
        page.prompt = Some(prompt);
    }
    effect.into_iter().map(OutMsg::Effect).collect()
}

fn commit_key(key: Key) -> Option<commit_picker::Msg> {
    match key {
        Key::Down | Key::Char('j') => Some(commit_picker::Msg::Move(1)),
        Key::Up | Key::Char('k') => Some(commit_picker::Msg::Move(-1)),
        _ => None,
    }
}

fn commit_move_msg(action: GlobalAction, step: u16) -> commit_picker::Msg {
    match action {
        GlobalAction::Top => commit_picker::Msg::ToTop,
        GlobalAction::Bottom => commit_picker::Msg::ToBottom,
        GlobalAction::PageUp => commit_picker::Msg::Page(-i32::from(step)),
        _ => commit_picker::Msg::Page(i32::from(step)),
    }
}

fn loaded(
    page: &mut Diff,
    request: DiffRequest,
    result: Result<Arc<[FileDiff]>, Box<EngineError>>,
) -> Vec<OutMsg> {
    match result {
        Ok(diffs) => {
            page.paging = Paging::Ready;
            page.base = request.base;
            page.target = request.target;
            page.mode = request.mode;
            page.scope = Scope::new(request.selection);
            page.prompt = None;
            if !matches!(page.view, DiffViewState::Range(_)) {
                page.view = DiffViewState::Range(Range::empty());
            }
            if let DiffViewState::Range(range) = &mut page.view {
                range.diffs = diffs;
                range.highlight.clear();
            }
            page.visible = Arc::from(diff_visible(page.diffs()));
            page.invalidate();
            Vec::new()
        }
        Err(error) => {
            page.paging = Paging::Ready;
            vec![OutMsg::Diagnose(error.to_string())]
        }
    }
}

fn history_loaded(
    page: &mut Diff,
    request: DiffRequest,
    result: Result<HistoryPayload, Box<EngineError>>,
) -> Vec<OutMsg> {
    let (steps, diffs) = match result {
        Ok(payload) => payload,
        Err(error) => {
            page.paging = Paging::Ready;
            return vec![OutMsg::Diagnose(error.to_string())];
        }
    };
    page.paging = Paging::Ready;
    page.base = request.base;
    page.target = request.target;
    page.mode = request.mode;
    page.scope = Scope::new(request.selection);
    page.prompt = None;
    let preferred = match &page.view {
        DiffViewState::Commits(commits) => commits
            .picker
            .steps_slice()
            .get(commits.picker.cursor())
            .map(|step| step.commit_id.clone()),
        DiffViewState::Range(_) => None,
    };
    page.view = DiffViewState::Commits(Commits {
        focus: CommitsFocus::Commits,
        picker: CommitPicker::steps(steps),
        body: DiffBody { scroll: 0 },
        diffs,
        highlight: Default::default(),
    });
    page.visible = Arc::from(diff_visible(page.diffs()));
    page.invalidate();

    // Restore the same commit as before, if it is still in range.
    let pending = if let DiffViewState::Commits(commits) = &page.view {
        preferred
            .and_then(|id| {
                commits
                    .picker
                    .steps_slice()
                    .iter()
                    .position(|step| step.commit_id == id)
            })
            .filter(|&index| index > 0)
            .and_then(|index| {
                commits
                    .picker
                    .steps_slice()
                    .get(index)
                    .cloned()
                    .map(|step| (index, step))
            })
    } else {
        None
    };
    if let Some((index, step)) = pending {
        page.paging = Paging::Loading;
        return vec![OutMsg::Effect(Cmd::Step {
            request: page.request(),
            index,
            step,
        })];
    }
    Vec::new()
}

fn step_loaded(
    page: &mut Diff,
    index: usize,
    inner_height: usize,
    result: Result<Arc<[FileDiff]>, Box<EngineError>>,
) -> Vec<OutMsg> {
    if !matches!(page.view, DiffViewState::Commits(_)) {
        return Vec::new();
    }
    match result {
        Ok(diffs) => {
            let len = match &page.view {
                DiffViewState::Commits(commits) => commits.picker.steps_slice().len(),
                DiffViewState::Range(_) => 0,
            };
            if index < len {
                page.paging = Paging::Ready;
                if let DiffViewState::Commits(commits) = &mut page.view {
                    commits.diffs = diffs;
                    commits.highlight.clear();
                    commits.picker.update(commit_picker::Msg::Loaded(index));
                    if inner_height > 0 {
                        commits.picker.set_scroll(commit_offset(
                            commits.picker.cursor(),
                            commits.picker.scroll(),
                            commits.picker.steps_slice().len(),
                            inner_height,
                        ));
                    }
                }
                page.visible = Arc::from(diff_visible(page.diffs()));
                page.invalidate();
            } else if let DiffViewState::Commits(commits) = &mut page.view {
                commits.picker.update(commit_picker::Msg::Failed);
            }
            Vec::new()
        }
        Err(error) => {
            page.paging = Paging::Ready;
            if let DiffViewState::Commits(commits) = &mut page.view {
                commits.picker.update(commit_picker::Msg::Failed);
            }
            vec![OutMsg::Diagnose(error.to_string())]
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Which side of the diff a pane draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Side {
    Old,
    New,
}

/// Draws one side of the diff.
pub(crate) fn view(
    page: &Diff,
    ctx: &ViewCtx,
    frame: &mut Frame,
    area: Rect,
    side: Side,
    focused: bool,
    edge: Edge,
) {
    let theme = ctx.theme;
    let (base, target) = page.revisions();
    let revision = match side {
        Side::Old => base,
        Side::New => target,
    };
    let title = pane_title(side, &revision, area.width);
    let block = pane_block(&title, focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(active) = page.active_diff() else {
        let (title, detail) = if ctx.busy && ctx.tree_empty {
            ("Loading", "Comparing projections…")
        } else if ctx.tree_empty {
            ("No changes", "Body-only edits are omitted.")
        } else {
            ("No file selected", "Select a file to view its diff.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    };
    let rows = page.rows();
    if rows.is_empty() {
        render_empty(
            frame,
            inner,
            "No text changes",
            "No projected text to compare for this file.",
            theme,
        );
        return;
    }
    let gutter = gutter_width(active);
    let content_width = (inner.width as usize).saturating_sub(gutter);
    let height = inner.height as usize;
    let skip = page.body_scroll() as usize;

    let mut lines = Vec::new();
    for row in rows.iter().skip(skip).take(height) {
        let (number, runs) = match side {
            Side::Old => (row.old_number, row.old_runs.as_slice()),
            Side::New => (row.new_number, row.new_runs.as_slice()),
        };
        let ranges = if row.continuation {
            Vec::new()
        } else {
            number.map_or_else(Vec::new, |line| page.search_ranges(search_side(side), line))
        };
        lines.push(diff_line(
            number,
            row.continuation,
            runs,
            row.kind,
            side,
            gutter,
            content_width,
            theme,
            &ranges,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draws the base/target revision prompt when it is open.
pub(crate) fn prompt_view(page: &Diff, theme: &Theme, frame: &mut Frame, area: Rect) {
    if let Some(prompt) = &page.prompt {
        let label = match prompt.side {
            DiffSide::Base => "base",
            DiffSide::Target => "target",
        };
        crate::component::text_input::render_prompt(frame, area, label, &prompt.input, theme);
    }
}

fn pane_title(side: Side, revision: &str, pane_width: u16) -> String {
    let label = match side {
        Side::Old => "BASE",
        Side::New => "TARGET",
    };
    // Leave room for the rounded border corners on a standalone pane. This
    // also keeps the title within the border in shared-divider layouts.
    let available = (pane_width as usize).saturating_sub(2);
    truncate_ellipsis(&format!(" {label} · {revision} "), available)
}

fn search_side(side: Side) -> SearchSide {
    match side {
        Side::Old => SearchSide::Old,
        Side::New => SearchSide::New,
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_line(
    number: Option<usize>,
    continuation: bool,
    runs: &[Run],
    kind: VisualRowKind,
    side: Side,
    gutter: usize,
    width: usize,
    theme: &Theme,
    search: &[(usize, usize, bool)],
) -> Line<'static> {
    let VisualRowKind::Diff(kind) = kind else {
        // Hunk headers and collapse indicators span the whole pane.
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        let full = gutter + width;
        let mut padded = if kind == VisualRowKind::Hunk {
            format!(" {text}")
        } else {
            text
        };
        let used = UnicodeWidthStr::width(padded.as_str());
        if used < full {
            padded.push_str(&" ".repeat(full - used));
        }
        let style = match kind {
            VisualRowKind::Hunk if theme.colors_enabled() => theme
                .fg_bg(theme.palette.hunk, theme.palette.surface_alt)
                .add_modifier(Modifier::BOLD),
            VisualRowKind::Hunk => Style::default(),
            _ => theme.fg(theme.palette.text_muted),
        };
        return Line::from(Span::styled(padded, style));
    };

    // Delta-style full-line background; the intra-line emphasis is already
    // baked into the run styles, so a run's own background wins over the base.
    let base = line_background(kind, side, theme);

    // Search matches are recolored on top of the diff emphasis.
    let owned;
    let runs: &[Run] = if search.is_empty() {
        runs
    } else {
        let mut styled = runs.to_vec();
        for (start, end, current) in search {
            let color = if *current {
                theme.color(theme.palette.match_current_bg)
            } else {
                theme.color(theme.palette.match_bg)
            };
            styled = highlight::apply_emphasis(&styled, &[(*start, *end)], color);
        }
        owned = styled;
        &owned
    };

    let field = gutter.saturating_sub(2);
    let sign = match (kind, side) {
        (DiffRowKind::Add, Side::New) | (DiffRowKind::Change, Side::New) => "+",
        (DiffRowKind::Delete, Side::Old) | (DiffRowKind::Change, Side::Old) => "-",
        _ => " ",
    };
    let number = match number {
        Some(number) => format!("{number:>field$}"),
        None if continuation => format!("{:>field$}", "…"),
        None => " ".repeat(field),
    };
    let sign_color = match sign {
        "+" => theme.palette.add_fg,
        "-" => theme.palette.del_fg,
        _ => theme.palette.gutter,
    };
    let mut marker_style = theme.fg(sign_color);
    if marker_style.bg.is_none() {
        marker_style.bg = base.bg;
    }
    let mut spans = vec![Span::styled(format!("{sign}{number} "), marker_style)];
    let mut used = 0usize;
    for run in runs {
        used += UnicodeWidthStr::width(run.text.as_str());
        let mut style = run.style;
        if style.bg.is_none() {
            style.bg = base.bg;
        }
        spans.push(Span::styled(run.text.clone(), style));
    }
    let padding = width.saturating_sub(used);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), base));
    }
    Line::from(spans)
}

fn line_background(kind: DiffRowKind, side: Side, theme: &Theme) -> Style {
    if !theme.colors_enabled() {
        return Style::default();
    }
    match (kind, side) {
        (DiffRowKind::Add, _) | (DiffRowKind::Change, Side::New) => theme.bg(theme.palette.add_bg),
        (DiffRowKind::Delete, _) | (DiffRowKind::Change, Side::Old) => {
            theme.bg(theme.palette.del_bg)
        }
        _ => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme::{Capability, Flavor};

    #[test]
    fn pane_titles_identify_sides_and_fit_narrow_borders() {
        assert_eq!(pane_title(Side::Old, "main", 20), " BASE · main ");
        assert_eq!(pane_title(Side::New, "feature", 20), " TARGET · feature ");

        let narrow = pane_title(Side::New, "a-long-feature-branch", 16);
        assert_eq!(narrow, " TARGET · a-l…");
        assert_eq!(UnicodeWidthStr::width(narrow.as_str()), 14);
        assert_eq!(pane_title(Side::Old, "main", 2), "");
    }

    #[test]
    fn hunk_header_fills_the_pane_with_a_subtle_band() {
        let theme = Theme::dark();
        let runs = vec![Run {
            style: Style::default(),
            text: "@@ -12,3 +12,4 @@".to_owned(),
        }];

        let line = diff_line(
            None,
            false,
            &runs,
            VisualRowKind::Hunk,
            Side::Old,
            4,
            24,
            &theme,
            &[],
        );
        let span = &line.spans[0];
        assert_eq!(UnicodeWidthStr::width(span.content.as_ref()), 28);
        assert!(span.content.starts_with(" @@ -12,3 +12,4 @@"));
        assert_eq!(span.style.fg, Some(theme.color(theme.palette.hunk)));
        assert_eq!(span.style.bg, Some(theme.color(theme.palette.surface_alt)));
    }

    #[test]
    fn hunk_header_keeps_its_range_markers_without_color() {
        let theme = Theme::new(Flavor::Dark, Capability::NoColor);
        let runs = vec![Run {
            style: Style::default(),
            text: "@@ -1,2 +1,3 @@".to_owned(),
        }];

        let line = diff_line(
            None,
            false,
            &runs,
            VisualRowKind::Hunk,
            Side::New,
            4,
            20,
            &theme,
            &[],
        );
        assert!(line.spans[0].content.starts_with(" @@ -1,2 +1,3 @@"));
        assert_eq!(line.spans[0].style, Style::default());
    }
}
