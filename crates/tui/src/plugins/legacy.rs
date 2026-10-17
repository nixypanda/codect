//! The `Legacy` plugin: everything the shell owned before the plugins were
//! extracted. It wraps `Loaded`, `Tree`, and the `Overlay` enum and delegates
//! to today's functions, so tasks 4.2–4.9 can peel one surface at a time.
//!
//! It declares the current panes and commands, routes pane input, and emits
//! kernel requests for every shared-state change (quit, focus, selection,
//! search, the overlay slot).

use std::sync::Arc;

use base::{AreaSet, DiffRowKind, FileDiff, RepoPath, SourceOutline};
use engine::{Engine, EngineError};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::action::{Action, CommitsAction, GlobalAction, RangeAction, ShowAction};
use crate::api::Cmd;
use crate::command;
use crate::component::commit_picker;
use crate::component::overlay::{self, Overlay};
use crate::component::tree::{self, RowKind, Tree};
use crate::kernel::chrome::{Chip, ChipTone, Segment, SegmentTone};
use crate::kernel::command::CommandSpec;
use crate::kernel::cx::Cx;
use crate::kernel::help::HelpEntry;
use crate::kernel::layout::PaneGeometry;
use crate::kernel::nav::{Nav, page_step};
use crate::kernel::plugin::{
    Event, HintTarget, Input, MouseInput, Out, OverlaySpec, PaneId, Plugin, PluginId, Reply,
    Request,
};
use crate::page::diff::Side;
use crate::page::{
    Commits, Ctx as PageCtx, Diff, DiffSide, DiffViewState, Loaded, OutMsg, Paging, Range, Show,
    ShowBody, ViewCtx, diff as diff_page, show as show_page,
};
use crate::plugin::{
    COMMITS_PANE, DIFF_BAND_FOCUS, NEW_PANE, OLD_PANE, SOURCE_PANE, TREE_FOCUS, TREE_PANE,
};
use crate::render::layout::{VisualRow, commit_offset};
use crate::route::{DiffRequest, DiffView, LoadRequest, ShowRequest};
use crate::util::input::{Key, MouseKind};
use crate::workspace::SearchMatch;

/// The plugin's id.
pub const LEGACY: PluginId = PluginId("legacy");

/// The one overlay slot's pane.
pub const LEGACY_OVERLAY: PaneId = PaneId {
    owner: LEGACY,
    slot: 0,
};

const MOUSE_SCROLL_STEP: i32 = 3;

/// The five effect results the runtime folds back.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum Msg {
    ShowLoaded {
        request: ShowRequest,
        result: Result<Arc<[SourceOutline]>, Box<EngineError>>,
    },
    DiffLoaded {
        request: DiffRequest,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
    HistoryLoaded {
        request: DiffRequest,
        result: Result<crate::api::HistoryPayload, Box<EngineError>>,
    },
    StepLoaded {
        index: usize,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

/// The legacy plugin's state.
#[derive(Clone)]
pub struct Legacy {
    pub loaded: Loaded,
    pub tree: Tree,
    pub overlay: Option<Overlay>,
}

impl Legacy {
    /// Build the plugin's initial state for a session.
    pub fn new(request: LoadRequest) -> Self {
        let loaded = match request {
            LoadRequest::Show {
                revision,
                mode,
                selection: _,
            } => Loaded::Show(Show {
                revision,
                loaded_mode: mode,
                files: Arc::from(Vec::new()),
                body: ShowBody {
                    scroll: 0,
                    hscroll: 0,
                    cursor_line: 1,
                },
                paging: Paging::Loading,
                highlight: Default::default(),
                folds: Default::default(),
                fold_state: Default::default(),
                visible_rows: Default::default(),
                prompt: None,
            }),
            LoadRequest::Diff {
                base,
                target,
                mode,
                selection: _,
                view,
                merge_base,
            } => Loaded::Diff(Diff::new(
                base,
                target,
                mode,
                match view {
                    DiffView::Range => DiffViewState::Range(Range::empty()),
                    DiffView::Commits => DiffViewState::Commits(Commits::empty()),
                },
                merge_base,
            )),
        };
        Self {
            loaded,
            tree: Tree::default(),
            overlay: None,
        }
    }

    // -- helpers ------------------------------------------------------------

    fn page_ctx(&self, cx: &Cx) -> PageCtx {
        PageCtx {
            height: cx.size.height,
            focus: cx.focus,
        }
    }

    fn tree_viewport_height(&self, cx: &Cx) -> usize {
        cx.layout_if_focused(TREE_FOCUS)
            .slots
            .iter()
            .find(|slot| slot.pane == TREE_PANE)
            .map_or(0, |slot| slot.geometry.inner.height as usize)
    }

    fn commits_inner(&self, cx: &Cx) -> Option<Rect> {
        cx.layout_if_focused(DIFF_BAND_FOCUS)
            .slots
            .iter()
            .find(|slot| slot.pane == COMMITS_PANE)
            .map(|slot| slot.geometry.inner)
    }

    fn route_show(&mut self, msg: show_page::Msg, cx: &Cx) -> Vec<Out<Cmd>> {
        let ctx = self.page_ctx(cx);
        if let Loaded::Show(show) = &mut self.loaded {
            convert(show_page::update(msg, show, cx.workspace, &ctx))
        } else {
            Vec::new()
        }
    }

    fn route_diff(&mut self, msg: diff_page::Msg, cx: &Cx) -> Vec<Out<Cmd>> {
        let ctx = self.page_ctx(cx);
        if let Loaded::Diff(diff) = &mut self.loaded {
            convert(diff_page::update(msg, diff, cx.workspace, &ctx))
        } else {
            Vec::new()
        }
    }

    fn route_page_action(&mut self, action: GlobalAction, cx: &Cx) -> Vec<Out<Cmd>> {
        match &self.loaded {
            Loaded::Show(_) => self.route_show(show_page::Msg::Action(action), cx),
            Loaded::Diff(_) => self.route_diff(diff_page::Msg::Action(action), cx),
        }
    }

    fn route_prompt(&mut self, key: Key, cx: &Cx) -> Vec<Out<Cmd>> {
        match &self.loaded {
            Loaded::Show(_) => self.route_show(show_page::Msg::Prompt(key), cx),
            Loaded::Diff(_) => self.route_diff(diff_page::Msg::Prompt(key), cx),
        }
    }

    fn route_reveal_match(&mut self, matched: SearchMatch, cx: &Cx) -> Vec<Out<Cmd>> {
        match &self.loaded {
            Loaded::Show(_) => self.route_show(show_page::Msg::RevealMatch(matched), cx),
            Loaded::Diff(_) => self.route_diff(diff_page::Msg::RevealMatch(matched), cx),
        }
    }

    fn route_requery(&mut self, change: crate::request::QueryChange, cx: &Cx) -> Vec<Out<Cmd>> {
        match &self.loaded {
            Loaded::Show(_) => self.route_show(show_page::Msg::Requery(change), cx),
            Loaded::Diff(_) => self.route_diff(diff_page::Msg::Requery(change), cx),
        }
    }

    /// Open `overlay`. The kernel derives the open slot from the plugin.
    fn open_overlay(&mut self, overlay: Overlay) -> Vec<Out<Cmd>> {
        self.overlay = Some(overlay);
        Vec::new()
    }

    fn open_palette(&mut self, cx: &Cx) -> Vec<Out<Cmd>> {
        let entries = self.loaded.palette_entries();
        let groups = self.loaded.prefix_groups();
        let palette = Overlay::palette(&overlay::Ctx {
            visible: &cx.workspace.files.paths,
            entries: &entries,
            groups: &groups,
            mode: cx.workspace.query.mode,
            pending_prefix: cx.pending_prefix,
        });
        self.open_overlay(palette)
    }

    fn open_finder(&mut self, cx: &Cx) -> Vec<Out<Cmd>> {
        let entries = self.loaded.palette_entries();
        let groups = self.loaded.prefix_groups();
        let finder = Overlay::finder(&overlay::Ctx {
            visible: &cx.workspace.files.paths,
            entries: &entries,
            groups: &groups,
            mode: cx.workspace.query.mode,
            pending_prefix: cx.pending_prefix,
        });
        self.open_overlay(finder)
    }

    pub(crate) fn apply_action(&mut self, action: Action, cx: &Cx) -> Vec<Out<Cmd>> {
        match action {
            Action::Global(GlobalAction::Help) => self.open_overlay(Overlay::help()),
            Action::Global(GlobalAction::Scope) => {
                let mut outs = self.open_overlay(Overlay::scope());
                outs.push(Out::Effect(Cmd::LoadAreas));
                outs
            }
            Action::Global(GlobalAction::Mode) => {
                self.open_overlay(Overlay::mode(cx.workspace.query.mode))
            }
            Action::Global(GlobalAction::Palette) => self.open_palette(cx),
            Action::Global(GlobalAction::Finder) => self.open_finder(cx),
            Action::Global(GlobalAction::Search) => self.open_overlay(Overlay::search()),
            Action::Global(GlobalAction::TreeWider) => {
                vec![Out::Kernel(Request::ResizeNavigator(
                    crate::request::NavigatorResize::Wider,
                ))]
            }
            Action::Global(GlobalAction::TreeNarrower) => {
                vec![Out::Kernel(Request::ResizeNavigator(
                    crate::request::NavigatorResize::Narrower,
                ))]
            }
            Action::Global(GlobalAction::TreeReset) => {
                vec![Out::Kernel(Request::ResizeNavigator(
                    crate::request::NavigatorResize::Reset,
                ))]
            }
            Action::Show(ShowAction::EditRevision) => {
                self.route_show(show_page::Msg::EditRevision, cx)
            }
            Action::Show(action) => self.route_show(show_page::Msg::Fold(action), cx),
            Action::Range(RangeAction::EditBase) => {
                self.route_diff(diff_page::Msg::EditSide(DiffSide::Base), cx)
            }
            Action::Range(RangeAction::EditTarget) => {
                self.route_diff(diff_page::Msg::EditSide(DiffSide::Target), cx)
            }
            Action::Range(RangeAction::SwitchToCommits) => {
                self.route_diff(diff_page::Msg::SwitchView(DiffView::Commits), cx)
            }
            Action::Commits(CommitsAction::SwitchToRange) => {
                self.route_diff(diff_page::Msg::SwitchView(DiffView::Range), cx)
            }
            // Kernel-owned global actions never reach a plugin.
            Action::Global(_) => Vec::new(),
        }
    }

    fn apply_overlay_outs(&mut self, outs: Vec<overlay::OutMsg>, cx: &Cx) -> Vec<Out<Cmd>> {
        let mut result = Vec::new();
        for out in outs {
            match out {
                overlay::OutMsg::Close => {
                    self.overlay = None;
                }
                overlay::OutMsg::Selection(selection) => {
                    result.push(Out::Kernel(Request::Requery(
                        crate::request::QueryChange::Scope(selection),
                    )));
                }
                overlay::OutMsg::Mode(mode) => {
                    result.push(Out::Kernel(Request::Requery(
                        crate::request::QueryChange::Mode(mode),
                    )));
                }
                overlay::OutMsg::Action(action) => result.extend(self.apply_action(action, cx)),
                overlay::OutMsg::File(path) => {
                    result.push(Out::Kernel(Request::Select(Some(path))));
                }
                overlay::OutMsg::Search(needle) => {
                    result.push(Out::Kernel(Request::SetSearch(needle)));
                }
                overlay::OutMsg::SearchCommit => {
                    result.push(Out::Kernel(Request::CommitSearch));
                }
            }
        }
        result
    }

    fn overlay_key(&mut self, key: Key, cx: &Cx) -> Reply<Cmd> {
        let entries = self.loaded.palette_entries();
        let groups = self.loaded.prefix_groups();
        let octx = overlay::Ctx {
            visible: &cx.workspace.files.paths,
            entries: &entries,
            groups: &groups,
            mode: cx.workspace.query.mode,
            pending_prefix: cx.pending_prefix,
        };
        let Some(overlay) = &mut self.overlay else {
            return Reply::unhandled();
        };
        match overlay.key(key, &octx) {
            Some(outs) => {
                let outs = self.apply_overlay_outs(outs, cx);
                Reply {
                    handled: true,
                    outs,
                }
            }
            None => Reply::unhandled(),
        }
    }

    fn tree_apply(&mut self, msg: tree::Msg, cx: &Cx) -> Vec<Out<Cmd>> {
        let height = self.tree_viewport_height(cx);
        let selected = self.tree.update(msg, &cx.workspace.files.paths, height);
        if let Some(tree::OutMsg::Selected(path)) = selected {
            vec![Out::Kernel(Request::Select(Some(path)))]
        } else {
            Vec::new()
        }
    }

    fn key_input(&mut self, pane: PaneId, key: Key, cx: &Cx) -> Reply<Cmd> {
        if pane == LEGACY_OVERLAY {
            if self.loaded.prompt_active() {
                let outs = self.route_prompt(key, cx);
                return Reply {
                    handled: true,
                    outs,
                };
            }
            return self.overlay_key(key, cx);
        }
        if pane == TREE_PANE {
            return match tree_key(key) {
                Some(msg) => {
                    let outs = self.tree_apply(msg, cx);
                    Reply {
                        handled: true,
                        outs,
                    }
                }
                None => Reply::unhandled(),
            };
        }
        if pane == SOURCE_PANE {
            if key == Key::Enter {
                let outs = self.route_show(show_page::Msg::Fold(ShowAction::ToggleFold), cx);
                return Reply {
                    handled: true,
                    outs,
                };
            }
            if matches!(
                key,
                Key::Down
                    | Key::Char('j')
                    | Key::Up
                    | Key::Char('k')
                    | Key::Right
                    | Key::Char('l')
                    | Key::Left
                    | Key::Char('h')
            ) {
                let outs = self.route_show(show_page::Msg::Key(key), cx);
                return Reply {
                    handled: true,
                    outs,
                };
            }
            return Reply::unhandled();
        }
        if pane == OLD_PANE || pane == NEW_PANE || pane == COMMITS_PANE {
            if matches!(key, Key::Down | Key::Char('j') | Key::Up | Key::Char('k')) {
                let outs = self.route_diff(diff_page::Msg::Key(key), cx);
                return Reply {
                    handled: true,
                    outs,
                };
            }
            return Reply::unhandled();
        }
        Reply::unhandled()
    }

    fn nav_input(&mut self, pane: PaneId, nav: Nav, cx: &Cx) -> Vec<Out<Cmd>> {
        let action = match nav {
            Nav::Top => GlobalAction::Top,
            Nav::Bottom => GlobalAction::Bottom,
            Nav::PageUp => GlobalAction::PageUp,
            Nav::PageDown => GlobalAction::PageDown,
        };
        if pane == TREE_PANE {
            let step = page_step(cx.size.height);
            return self.tree_apply(tree_nav_msg(action, step), cx);
        }
        self.route_page_action(action, cx)
    }

    fn mouse_input(&mut self, pane: PaneId, event: MouseInput, cx: &Cx) -> Vec<Out<Cmd>> {
        let inner = event.geometry.inner;
        let point = ratatui::layout::Position {
            x: event.column,
            y: event.row,
        };
        if pane == COMMITS_PANE {
            let msg = match event.kind {
                MouseKind::Click => {
                    if !inner.contains(point) {
                        return Vec::new();
                    }
                    self.commit_click_index(inner, event.row)
                        .map(commit_picker::Msg::Select)
                }
                MouseKind::ScrollUp => Some(commit_picker::Msg::Move(-MOUSE_SCROLL_STEP)),
                MouseKind::ScrollDown => Some(commit_picker::Msg::Move(MOUSE_SCROLL_STEP)),
            };
            return match msg {
                Some(msg) => self.route_diff(diff_page::Msg::Commit(msg), cx),
                None => Vec::new(),
            };
        }
        if pane == TREE_PANE {
            return match event.kind {
                MouseKind::Click => {
                    let index = self.tree.scroll + usize::from(event.row.saturating_sub(inner.y));
                    if index < self.tree.rows.len() {
                        self.tree_apply(tree::Msg::Click(index), cx)
                    } else {
                        Vec::new()
                    }
                }
                MouseKind::ScrollUp => self.tree_apply(tree::Msg::Scroll(-MOUSE_SCROLL_STEP), cx),
                MouseKind::ScrollDown => self.tree_apply(tree::Msg::Scroll(MOUSE_SCROLL_STEP), cx),
            };
        }
        if pane == SOURCE_PANE {
            return match event.kind {
                MouseKind::Click => {
                    if inner.contains(point)
                        && let Loaded::Show(show) = &self.loaded
                    {
                        let row = show.body.scroll as usize
                            + usize::from(event.row.saturating_sub(inner.y));
                        if row < show.line_count() {
                            let marker = event.column - inner.x < show_page::MARKER_CLICK_WIDTH;
                            return self.route_show(show_page::Msg::BodyClick { row, marker }, cx);
                        }
                    }
                    Vec::new()
                }
                MouseKind::ScrollUp | MouseKind::ScrollDown => {
                    let delta = if event.kind == MouseKind::ScrollUp {
                        -MOUSE_SCROLL_STEP
                    } else {
                        MOUSE_SCROLL_STEP
                    };
                    self.route_show(show_page::Msg::Scroll(delta), cx)
                }
            };
        }
        if pane == OLD_PANE || pane == NEW_PANE {
            let delta = match event.kind {
                MouseKind::ScrollUp => -MOUSE_SCROLL_STEP,
                MouseKind::ScrollDown => MOUSE_SCROLL_STEP,
                MouseKind::Click => 0,
            };
            return if delta != 0 {
                self.route_diff(diff_page::Msg::Scroll(delta), cx)
            } else {
                Vec::new()
            };
        }
        Vec::new()
    }

    fn commit_click_index(&self, inner: Rect, row: u16) -> Option<usize> {
        let Loaded::Diff(diff) = &self.loaded else {
            return None;
        };
        let DiffViewState::Commits(commits) = &diff.view else {
            return None;
        };
        let offset = commit_offset(
            commits.picker.cursor(),
            commits.picker.scroll(),
            commits.picker.steps_slice().len(),
            inner.height as usize,
        );
        let index = offset + usize::from(row.saturating_sub(inner.y));
        (index < commits.picker.steps_slice().len()).then_some(index)
    }

    fn step_in_range(&self, index: usize) -> bool {
        match &self.loaded {
            Loaded::Diff(diff) => match &diff.view {
                DiffViewState::Commits(commits) => index < commits.picker.steps_slice().len(),
                DiffViewState::Range(_) => false,
            },
            Loaded::Show(_) => false,
        }
    }

    fn on_files_changed(&mut self, cx: &Cx) -> Vec<Out<Cmd>> {
        let hint = self.tree.cursor;
        self.tree.sync(&cx.workspace.files.paths);
        match cx.workspace.selected.clone() {
            Some(path) => {
                let height = self.tree_viewport_height(cx);
                self.tree.reveal(&path, height);
                Vec::new()
            }
            None => match nearest_visible(&self.tree, hint) {
                Some(path) => vec![Out::Kernel(Request::Select(Some(path)))],
                None => {
                    if let Some(index) = self.tree.first_file_row() {
                        self.tree.cursor = index;
                    }
                    Vec::new()
                }
            },
        }
    }

    fn on_selection_changed(&mut self, cx: &Cx) {
        match cx.workspace.selected.as_ref() {
            Some(path) => {
                let height = self.tree_viewport_height(cx);
                self.tree.reveal(path, height);
                self.loaded.reset_body_scroll(Some(path));
            }
            None => self.loaded.reset_body_scroll(None),
        }
    }

    fn diff_rows(&self) -> &[VisualRow] {
        self.loaded.diff_rows()
    }

    fn diff_stats(&self) -> Option<(usize, usize)> {
        if !matches!(self.loaded, Loaded::Diff(_)) {
            return None;
        }
        let mut added = 0;
        let mut removed = 0;
        for row in self.diff_rows() {
            if row.continuation {
                continue;
            }
            match row.kind {
                crate::render::layout::VisualRowKind::Diff(DiffRowKind::Add) => added += 1,
                crate::render::layout::VisualRowKind::Diff(DiffRowKind::Delete) => removed += 1,
                crate::render::layout::VisualRowKind::Diff(DiffRowKind::Change) => {
                    added += 1;
                    removed += 1;
                }
                crate::render::layout::VisualRowKind::Diff(DiffRowKind::Equal)
                | crate::render::layout::VisualRowKind::Hunk
                | crate::render::layout::VisualRowKind::Collapse(_) => {}
            }
        }
        Some((added, removed))
    }
}

impl Plugin for Legacy {
    const ID: PluginId = LEGACY;
    type Msg = Msg;
    type Cmd = Cmd;

    fn panes(&self, _cx: &Cx) -> Vec<crate::kernel::layout::PaneSpec> {
        self.loaded.panes()
    }

    /// The cheap, allocation-free focus membership: the kernel normalises focus
    /// on every update step, so it must not collect the panes.
    fn has_focus(&self, focus: crate::kernel::plugin::FocusKey) -> bool {
        self.loaded.focus_keys().contains(&focus)
    }

    /// The focus stop normalisation falls back to: the loaded kind's first key.
    fn first_focus(&self) -> Option<crate::kernel::plugin::FocusKey> {
        self.loaded.focus_keys().first().copied()
    }

    fn commands(&self, cx: &Cx) -> Vec<CommandSpec> {
        command::commands(&self.loaded, cx.focus)
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        Vec::new()
    }

    fn hints(&self, target: HintTarget, _cx: &Cx) -> Option<&'static str> {
        if self.loaded.prompt_active() {
            return Some("Enter apply   Esc cancel ");
        }
        match target {
            HintTarget::Overlay(_) => match &self.overlay {
                Some(Overlay::Scope(_)) => Some("↑↓ choose   Enter select   Esc close "),
                Some(Overlay::Mode { .. }) => Some("↑↓ choose   Enter apply   Esc close "),
                Some(Overlay::Palette(_)) | Some(Overlay::Finder(_)) => {
                    Some("type to filter   ↑↓ choose   Enter open   Esc close ")
                }
                Some(Overlay::Search(_)) => Some("type to search   Enter next   Esc cancel "),
                Some(Overlay::WhichKey { .. }) => {
                    Some("↑↓ / Ctrl-K/J choose   Enter run   Esc close ")
                }
                Some(Overlay::Help) => Some("Esc close "),
                None => Some("Esc close "),
            },
            HintTarget::Focus(focus) => {
                if focus == DIFF_BAND_FOCUS {
                    return Some("j/k choose commit   Tab files   Ctrl-P commands   ? help ");
                }
                if focus == TREE_FOCUS {
                    return Some(
                        "click open   Ctrl-P commands   Ctrl-F find   Tab content   ? help ",
                    );
                }
                Some(match self.loaded {
                    Loaded::Show(_) => {
                        "wheel / j k scroll   / search   Ctrl-P commands   Tab tree   ? help "
                    }
                    Loaded::Diff(_) => {
                        "wheel / j k scroll   / search   n/N match   Ctrl-P commands   ? help "
                    }
                })
            }
        }
    }

    fn chips(&self, _cx: &Cx) -> Vec<Chip> {
        match &self.loaded {
            Loaded::Show(show) => vec![Chip {
                label: show.revision.clone(),
                tone: ChipTone::Neutral,
            }],
            Loaded::Diff(diff) => {
                let (base, target) = diff.revisions();
                vec![Chip {
                    label: format!("{base}..{target}"),
                    tone: ChipTone::Neutral,
                }]
            }
        }
    }

    fn status(&self, cx: &Cx) -> Vec<Segment> {
        if let Some((added, removed)) = self.diff_stats() {
            return vec![
                Segment {
                    text: format!("+{added} "),
                    tone: SegmentTone::Added,
                },
                Segment {
                    text: format!("−{removed} "),
                    tone: SegmentTone::Removed,
                },
            ];
        }
        if let Loaded::Show(show) = &self.loaded
            && show.active_source(cx.workspace.selected.as_ref()).is_some()
        {
            return vec![Segment {
                text: format!(
                    "{} of {} lines ",
                    show.line_count(),
                    show.source_line_count(cx.workspace.selected.as_ref())
                ),
                tone: SegmentTone::Muted,
            }];
        }
        Vec::new()
    }

    fn overlay_spec(&self, _pane: PaneId) -> OverlaySpec {
        if self.loaded.prompt_active() {
            return OverlaySpec {
                captures_text: true,
                modal: true,
                keeps_ctrl_jk: false,
            };
        }
        match &self.overlay {
            Some(overlay) => OverlaySpec {
                captures_text: overlay.captures_text(),
                modal: overlay.modal(),
                keeps_ctrl_jk: overlay.keeps_ctrl_jk(),
            },
            None => OverlaySpec::MODAL,
        }
    }

    fn overlay_pane(&self) -> Option<PaneId> {
        if self.loaded.prompt_active() || self.overlay.is_some() {
            Some(LEGACY_OVERLAY)
        } else {
            None
        }
    }

    fn busy(&self) -> bool {
        self.loaded.is_loading()
    }

    fn render(
        &self,
        pane: PaneId,
        geometry: PaneGeometry,
        focused: bool,
        cx: &Cx,
        frame: &mut Frame,
    ) {
        let ctx = crate::component::RenderCtx {
            theme: cx.theme,
            icons: cx.icons,
            workspace: cx.workspace,
            busy: cx.busy,
        };
        let vctx = ViewCtx {
            theme: cx.theme,
            busy: cx.busy,
            tree_empty: cx.workspace.files.paths.is_empty(),
        };
        let outer = geometry.outer;
        let edge = geometry.edge;
        if pane == TREE_PANE {
            tree::render(&self.tree, &ctx, frame, outer, focused, edge);
        } else if pane == SOURCE_PANE {
            if let Loaded::Show(show) = &self.loaded {
                show_page::view(show, cx.workspace, &vctx, frame, outer, focused, edge);
            }
        } else if pane == OLD_PANE {
            if let Loaded::Diff(diff) = &self.loaded {
                diff_page::view(
                    diff,
                    cx.workspace,
                    &vctx,
                    frame,
                    outer,
                    Side::Old,
                    focused,
                    edge,
                );
            }
        } else if pane == NEW_PANE {
            if let Loaded::Diff(diff) = &self.loaded {
                diff_page::view(
                    diff,
                    cx.workspace,
                    &vctx,
                    frame,
                    outer,
                    Side::New,
                    focused,
                    edge,
                );
            }
        } else if pane == COMMITS_PANE
            && let Loaded::Diff(diff) = &self.loaded
            && let DiffViewState::Commits(commits) = &diff.view
        {
            commit_picker::render(&commits.picker, &ctx, frame, outer, focused, edge);
        }
    }

    fn render_overlay(&self, _pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if self.loaded.prompt_active() {
            match &self.loaded {
                Loaded::Show(show) => show_page::prompt_view(show, cx.theme, frame, area),
                Loaded::Diff(diff) => diff_page::prompt_view(diff, cx.theme, frame, area),
            }
            return;
        }
        let Some(state) = &self.overlay else {
            return;
        };
        let entries = self.loaded.palette_entries();
        let groups = self.loaded.prefix_groups();
        let rctx = overlay::RenderCtx {
            theme: cx.theme,
            mode: cx.workspace.query.mode,
            entries: &entries,
            groups: &groups,
            visible: &cx.workspace.files.paths,
            matches: cx
                .workspace
                .search
                .as_ref()
                .map_or(0, |search| search.matches.len()),
            pending_prefix: cx.pending_prefix,
        };
        overlay::render(state, &rctx, frame, area);
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Cmd> {
        match input {
            Input::Key(key) => self.key_input(pane, key, cx),
            Input::Nav(nav) => Reply {
                handled: true,
                outs: self.nav_input(pane, nav, cx),
            },
            Input::Mouse(event) => Reply {
                handled: true,
                outs: self.mouse_input(pane, event, cx),
            },
        }
    }

    fn run(&mut self, command: crate::kernel::command::CommandId, cx: &Cx) -> Vec<Out<Cmd>> {
        match command::action_for(command.name) {
            Some(action) => self.apply_action(action, cx),
            None => Vec::new(),
        }
    }

    fn update(&mut self, msg: Msg, cx: &Cx) -> Vec<Out<Cmd>> {
        match msg {
            Msg::ShowLoaded { request, result } => {
                let installed = result.is_ok();
                let mut outs = self.route_show(show_page::Msg::Loaded(request, result), cx);
                if installed {
                    outs.push(Out::Kernel(Request::ClearDiagnostic));
                    outs.push(Out::Kernel(Request::CancelPrefix));
                }
                outs
            }
            Msg::DiffLoaded { request, result } => {
                let installed = result.is_ok();
                let mut outs = self.route_diff(diff_page::Msg::Loaded(request, result), cx);
                if installed {
                    outs.push(Out::Kernel(Request::ClearDiagnostic));
                    outs.push(Out::Kernel(Request::CancelPrefix));
                }
                outs
            }
            Msg::HistoryLoaded { request, result } => {
                let installed = result.is_ok();
                let mut outs = self.route_diff(diff_page::Msg::HistoryLoaded(request, result), cx);
                if installed {
                    outs.push(Out::Kernel(Request::ClearDiagnostic));
                }
                outs
            }
            Msg::StepLoaded { index, result } => {
                let inner_height = self
                    .commits_inner(cx)
                    .map_or(0, |rect| rect.height as usize);
                let installed = result.is_ok() && self.step_in_range(index);
                let mut outs = self.route_diff(
                    diff_page::Msg::StepLoaded {
                        index,
                        inner_height,
                        result,
                    },
                    cx,
                );
                if installed {
                    outs.push(Out::Kernel(Request::ClearDiagnostic));
                }
                outs
            }
            Msg::AreasLoaded(result) => {
                let outs = if let Some(overlay) = &mut self.overlay {
                    overlay.update(overlay::Msg::AreasLoaded(result))
                } else {
                    Vec::new()
                };
                self.apply_overlay_outs(outs, cx)
            }
        }
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> Vec<Out<Cmd>> {
        match event {
            Event::FilesChanged => self.on_files_changed(cx),
            Event::SelectionChanged => {
                self.on_selection_changed(cx);
                Vec::new()
            }
            Event::QueryChanged | Event::SearchChanged => Vec::new(),
            Event::PrefixPending(_) => self.open_overlay(Overlay::which_key()),
            Event::PrefixCleared => {
                if matches!(self.overlay, Some(Overlay::WhichKey { .. })) {
                    self.overlay = None;
                }
                Vec::new()
            }
            Event::Resized => {
                self.loaded
                    .clamp_scroll(cx.size.height, cx.workspace.selected.as_ref());
                Vec::new()
            }
            Event::OverlayClosed(pane) => {
                if *pane == LEGACY_OVERLAY && !self.loaded.prompt_active() {
                    self.overlay = None;
                }
                Vec::new()
            }
            Event::RevealMatch(matched) => self.route_reveal_match(*matched, cx),
            Event::Requery(change) => self.route_requery(change.clone(), cx),
        }
    }

    fn search(&mut self, needle: &str, cx: &Cx) -> Vec<SearchMatch> {
        match &mut self.loaded {
            Loaded::Show(show) => {
                show.ensure_visible_rows(cx.workspace.selected.as_ref());
                show.search_matches(needle, cx.workspace)
            }
            Loaded::Diff(diff) => diff.search_matches(needle, cx.workspace),
        }
    }

    fn after_batch(&mut self, pending: &[Cmd], cx: &Cx) -> Vec<Out<Cmd>> {
        let Loaded::Diff(diff) = &self.loaded else {
            return Vec::new();
        };
        if !matches!(diff.view, DiffViewState::Commits(_)) {
            return Vec::new();
        }
        if pending.iter().any(|cmd| matches!(cmd, Cmd::Diff(_))) {
            return Vec::new();
        }
        let pending_step = if let DiffViewState::Commits(commits) = &diff.view {
            commits.picker.target().and_then(|index| {
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
        let Some((index, step)) = pending_step else {
            return Vec::new();
        };
        let request = diff.request(cx.workspace.query.scope.selection.clone());
        if let Loaded::Diff(diff) = &mut self.loaded {
            diff.paging = Paging::Loading;
        }
        vec![Out::Effect(Cmd::Step {
            request,
            index,
            step,
        })]
    }

    fn settle(&mut self, cx: &Cx) {
        let visible_body = cx.body_visible();
        let selected = cx.workspace.selected.clone();
        self.loaded
            .settle_rows(cx.theme, visible_body, selected.as_ref());
        self.loaded.refresh_rows(
            cx.size.width,
            cx.size.height,
            cx.navigator_percent,
            cx.theme,
            selected.as_ref(),
        );
        let height = self.tree_viewport_height(cx);
        self.tree.fit_viewport(height);
    }

    fn perform(engine: &Engine, cmd: Cmd) -> Msg {
        interpret(engine, cmd)
    }

    fn startup_failure(msg: Msg) -> Result<Msg, Box<EngineError>> {
        match msg {
            Msg::ShowLoaded {
                result: Err(error), ..
            }
            | Msg::DiffLoaded {
                result: Err(error), ..
            }
            | Msg::HistoryLoaded {
                result: Err(error), ..
            } => Err(error),
            other => Ok(other),
        }
    }
}

// -- free helpers -----------------------------------------------------------

fn convert(outs: Vec<OutMsg>) -> Vec<Out<Cmd>> {
    outs.into_iter()
        .map(|out| match out {
            OutMsg::Effect(cmd) => Out::Effect(cmd),
            OutMsg::Request(request) => Out::Kernel(request),
        })
        .collect()
}

fn tree_key(key: Key) -> Option<tree::Msg> {
    match key {
        Key::Down | Key::Char('j') => Some(tree::Msg::Move(1)),
        Key::Up | Key::Char('k') => Some(tree::Msg::Move(-1)),
        Key::Right | Key::Char('l') => Some(tree::Msg::Open),
        Key::Left | Key::Char('h') => Some(tree::Msg::Close),
        Key::Enter => Some(tree::Msg::Toggle),
        _ => None,
    }
}

fn tree_nav_msg(action: GlobalAction, step: u16) -> tree::Msg {
    match action {
        GlobalAction::Top => tree::Msg::ToTop,
        GlobalAction::Bottom => tree::Msg::ToBottom,
        GlobalAction::PageUp => tree::Msg::Page(-i32::from(step)),
        _ => tree::Msg::Page(i32::from(step)),
    }
}

fn nearest_visible(tree: &Tree, hint: usize) -> Option<RepoPath> {
    let file_rows: Vec<usize> = tree
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| matches!(row.kind, RowKind::File { .. }))
        .map(|(index, _)| index)
        .collect();
    let chosen = file_rows
        .iter()
        .copied()
        .find(|&index| index >= hint)
        .or_else(|| file_rows.last().copied())?;
    Some(tree.rows[chosen].kind.path().clone())
}

/// The old `interpret`: run an effect against the engine and build its message.
fn interpret(engine: &Engine, cmd: Cmd) -> Msg {
    match cmd {
        Cmd::Show(request) => {
            let result = engine
                .show_source_outlines(&request.revision, request.mode, &request.selection)
                .map(Arc::from)
                .map_err(Box::new);
            Msg::ShowLoaded { request, result }
        }
        Cmd::Diff(request) => match request.view {
            DiffView::Range => {
                let base = if request.merge_base {
                    engine::DiffBase::MergeBase
                } else {
                    engine::DiffBase::Given
                };
                let result = engine
                    .diff(
                        &request.base,
                        &request.target,
                        request.mode,
                        &request.selection,
                        base,
                    )
                    .map(Arc::from)
                    .map_err(Box::new);
                Msg::DiffLoaded { request, result }
            }
            DiffView::Commits => {
                let result = engine
                    .first_parent_steps(&request.base, &request.target)
                    .and_then(|steps| {
                        let diffs: Vec<_> = match steps.first() {
                            Some(step) => engine.diff(
                                &step.parent_id.to_string(),
                                &step.commit_id.to_string(),
                                request.mode,
                                &request.selection,
                                engine::DiffBase::Given,
                            )?,
                            None => Vec::new(),
                        };
                        Ok((Arc::from(steps), Arc::from(diffs)))
                    })
                    .map_err(Box::new);
                Msg::HistoryLoaded { request, result }
            }
        },
        Cmd::Step {
            request,
            index,
            step,
        } => {
            let result = engine
                .diff(
                    &step.parent_id.to_string(),
                    &step.commit_id.to_string(),
                    request.mode,
                    &request.selection,
                    engine::DiffBase::Given,
                )
                .map(Arc::from)
                .map_err(Box::new);
            Msg::StepLoaded { index, result }
        }
        Cmd::LoadAreas => Msg::AreasLoaded(engine.load_areas().map_err(Box::new)),
    }
}
