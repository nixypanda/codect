//! The diff plugin: the compare (range) and history (commits) views.
//!
//! One plugin owns both views because they share the base, the target, the
//! merge-base flag, and the diff body; switching between them is internal. The
//! panes `OLD` and `NEW` share one focus stop; the commits band has its own.
//!
//! The plugin never owns the mode, the scope, the selected path, or the search:
//! those live on the kernel's [`tui_core::workspace::Workspace`].

use std::collections::BTreeMap;
use std::sync::Arc;

use base::{DiffRowKind, FileDiff, ProjectionMode, RepoPath, Selection};
use engine::{CommitStep, Engine, EngineError};
use ratatui::Frame;

use crate::DiffView;
use crate::{COMMITS_PANE, DIFF, DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, NEW_PANE, OLD_PANE};
use tui_core::chrome::{Chip, ChipTone, Segment, SegmentTone};
use tui_core::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq};
use tui_core::cx::Cx;
use tui_core::help::{HelpEntry, MOUSE, VIEW};
use tui_core::layout::{PaneGeometry, PaneSpec, Placement};
use tui_core::nav::{Nav, page_step, scroll_by};
use tui_core::plugin::{
    Event, FocusKey, HintTarget, Input, MouseInput, Out, PaneId, Plugin, PluginId, QueryChange,
    Reply, Request,
};
use tui_core::search::SearchMatch;
use tui_core::util::input::{Key, MouseKind};
use tui_core::workspace::{BadgeTone, EmptyText, FileSet, NO_CHANGES, Query, Scope};

use super::DiffRequest;
use super::commit_picker::{self, CommitPicker};
use super::layout::{VisualRowKind, commit_offset, diff_panes};
use super::model::{Commits, DiffBody, DiffRows, DiffViewState, Range, diff_visible};
use super::pane::{self, Side};

const MOUSE_SCROLL_STEP: i32 = 3;

/// The tag the base-revision prompt echoes back with its value.
pub const BASE_PROMPT_TAG: u8 = 0;

/// The tag the target-revision prompt echoes back with its value.
pub const TARGET_PROMPT_TAG: u8 = 1;

pub type HistoryPayload = (Arc<[CommitStep]>, Arc<[FileDiff]>);

pub const OLD_SPEC: PaneSpec = PaneSpec {
    id: OLD_PANE,
    placement: Placement::Body,
    focus: DIFF_BODY_FOCUS,
};

pub const NEW_SPEC: PaneSpec = PaneSpec {
    id: NEW_PANE,
    placement: Placement::Body,
    focus: DIFF_BODY_FOCUS,
};

pub const COMMITS_SPEC: PaneSpec = PaneSpec {
    id: COMMITS_PANE,
    placement: Placement::Band,
    focus: DIFF_BAND_FOCUS,
};

/// The diff plugin's effect results.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum Msg {
    DiffLoaded {
        request: DiffRequest,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
    HistoryLoaded {
        request: DiffRequest,
        result: Result<HistoryPayload, Box<EngineError>>,
    },
    StepLoaded {
        index: usize,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
}

/// The diff plugin's effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    Range(DiffRequest),
    History(DiffRequest),
    Step {
        request: DiffRequest,
        index: usize,
        step: CommitStep,
    },
}

/// The compare/history plugin's state.
#[derive(Clone, Debug)]
pub struct DiffPlugin {
    pub base: String,
    pub target: String,
    pub loaded_mode: ProjectionMode,
    pub merge_base: bool,
    pub view: DiffViewState,
    pub loading: bool,
    pub generation: u64,
    pub rows: Arc<DiffRows>,
    active: bool,
}

impl DiffPlugin {
    /// Build the plugin for a diff session. The startup load fills it in.
    pub fn new(
        base: String,
        target: String,
        mode: ProjectionMode,
        view: DiffView,
        merge_base: bool,
    ) -> Self {
        let view = match view {
            DiffView::Range => DiffViewState::Range(Range::empty()),
            DiffView::Commits => DiffViewState::Commits(Commits::empty()),
        };
        Self {
            base,
            target,
            loaded_mode: mode,
            merge_base,
            view,
            loading: true,
            generation: 0,
            rows: Arc::new(DiffRows::default()),
            active: true,
        }
    }

    /// Build an inactive plugin for a session of another kind.
    pub fn inactive() -> Self {
        Self {
            base: String::new(),
            target: String::new(),
            loaded_mode: ProjectionMode::Types,
            merge_base: false,
            view: DiffViewState::Range(Range::empty()),
            loading: false,
            generation: 0,
            rows: Arc::new(DiffRows::default()),
            active: false,
        }
    }

    /// The load request for `selection`, projected with the currently loaded
    /// mode.
    pub fn request(&self, selection: Selection) -> DiffRequest {
        DiffRequest {
            base: self.base.clone(),
            target: self.target.clone(),
            mode: self.loaded_mode,
            selection,
            view: match &self.view {
                DiffViewState::Range(_) => DiffView::Range,
                DiffViewState::Commits(_) => DiffView::Commits,
            },
            merge_base: self.merge_base,
        }
    }

    fn range(&self) -> bool {
        matches!(self.view, DiffViewState::Range(_))
    }

    fn commits(&self) -> bool {
        matches!(self.view, DiffViewState::Commits(_))
    }

    /// The commit picker's inner height in the layout focused on the band. This
    /// is today's `commits_inner`.
    fn commits_inner(&self, cx: &Cx) -> Option<ratatui::layout::Rect> {
        cx.layout_if_focused(DIFF_BAND_FOCUS)
            .slots
            .iter()
            .find(|slot| slot.pane == COMMITS_PANE)
            .map(|slot| slot.geometry.inner)
    }

    fn commit_click_index(&self, inner: ratatui::layout::Rect, row: u16) -> Option<usize> {
        let DiffViewState::Commits(commits) = &self.view else {
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

    fn switch_view(&mut self, view: DiffView, cx: &Cx) -> Vec<Out<Cmd>> {
        let differs = matches!(
            (&self.view, view),
            (DiffViewState::Range(_), DiffView::Commits)
                | (DiffViewState::Commits(_), DiffView::Range)
        );
        if !differs {
            return Vec::new();
        }
        if view == DiffView::Commits
            && (engine::is_snapshot_spec(&self.base) || engine::is_snapshot_spec(&self.target))
        {
            return vec![Out::Kernel(Request::Diagnose(
                "The commits view needs commit revisions".to_owned(),
            ))];
        }
        if view == DiffView::Commits && self.merge_base {
            return vec![Out::Kernel(Request::Diagnose(
                "The commits view cannot use a merge-base base".to_owned(),
            ))];
        }
        self.loading = true;
        let mut request = self.request(cx.workspace.query.scope.selection.clone());
        request.view = view;
        vec![Out::Effect(match view {
            DiffView::Range => Cmd::Range(request),
            DiffView::Commits => Cmd::History(request),
        })]
    }

    fn prompt_submitted(&mut self, tag: u8, value: String, cx: &Cx) -> Vec<Out<Cmd>> {
        if value.is_empty() {
            return Vec::new();
        }
        let side = match tag {
            BASE_PROMPT_TAG => Side::Old,
            TARGET_PROMPT_TAG => Side::New,
            _ => return Vec::new(),
        };
        let mut request = self.request(cx.workspace.query.scope.selection.clone());
        match side {
            Side::Old => request.base = value,
            Side::New => request.target = value,
        }
        self.loading = true;
        let cmd = match &self.view {
            DiffViewState::Range(_) => Cmd::Range(request),
            DiffViewState::Commits(_) => Cmd::History(request),
        };
        vec![Out::Effect(cmd)]
    }

    fn diff_stats(&self) -> (usize, usize) {
        let mut added = 0;
        let mut removed = 0;
        for row in self.rows() {
            if row.continuation {
                continue;
            }
            match row.kind {
                VisualRowKind::Diff(DiffRowKind::Add) => added += 1,
                VisualRowKind::Diff(DiffRowKind::Delete) => removed += 1,
                VisualRowKind::Diff(DiffRowKind::Change) => {
                    added += 1;
                    removed += 1;
                }
                VisualRowKind::Diff(DiffRowKind::Equal)
                | VisualRowKind::Hunk
                | VisualRowKind::Collapse(_) => {}
            }
        }
        (added, removed)
    }
}

fn commands(model: &DiffPlugin) -> Vec<CommandSpec> {
    let range = model.range();
    let switchable = range
        && !engine::is_snapshot_spec(&model.base)
        && !engine::is_snapshot_spec(&model.target)
        && !model.merge_base;
    vec![
        CommandSpec {
            id: CommandId {
                owner: DIFF,
                name: "base.edit",
            },
            label: "Edit base revision",
            hint: "b",
            bindings: vec![Binding {
                seq: KeySeq::Key(Key::Char('b')),
                scope: KeyScope::Anywhere,
            }],
            palette: range,
            group: None,
            rank: 200,
        },
        CommandSpec {
            id: CommandId {
                owner: DIFF,
                name: "target.edit",
            },
            label: "Edit target revision",
            hint: "t",
            bindings: vec![Binding {
                seq: KeySeq::Key(Key::Char('t')),
                scope: KeyScope::Anywhere,
            }],
            palette: range,
            group: None,
            rank: 201,
        },
        CommandSpec {
            id: CommandId {
                owner: DIFF,
                name: "view.commits",
            },
            label: "Switch to commits view",
            hint: "",
            bindings: Vec::new(),
            palette: switchable,
            group: None,
            rank: 202,
        },
        CommandSpec {
            id: CommandId {
                owner: DIFF,
                name: "view.range",
            },
            label: "Switch to range view",
            hint: "",
            bindings: Vec::new(),
            palette: model.commits(),
            group: None,
            rank: 200,
        },
    ]
}

fn help(model: &DiffPlugin) -> Vec<HelpEntry> {
    let mut rows = vec![HelpEntry {
        section: VIEW,
        keys: "b / t",
        text: "edit the base or target",
        rank: 50,
    }];
    if model.commits() {
        rows.push(HelpEntry {
            section: MOUSE,
            keys: "click commit",
            text: "select a commit and show its diff",
            rank: 25,
        });
    }
    rows
}

impl Plugin for DiffPlugin {
    const ID: PluginId = DIFF;
    type Msg = Msg;
    type Cmd = Cmd;

    fn panes(&self, cx: &Cx) -> Vec<PaneSpec> {
        if !cx.is_active_view(DIFF) {
            return Vec::new();
        }
        diff_panes(self.commits())
    }

    fn has_focus(&self, focus: FocusKey) -> bool {
        if !self.active {
            return false;
        }
        focus == DIFF_BODY_FOCUS || (focus == DIFF_BAND_FOCUS && self.commits())
    }

    fn first_focus(&self) -> Option<FocusKey> {
        self.active.then_some(DIFF_BODY_FOCUS)
    }

    fn commands(&self, cx: &Cx) -> Vec<CommandSpec> {
        if cx.is_active_view(DIFF) {
            commands(self)
        } else {
            Vec::new()
        }
    }

    fn help(&self, cx: &Cx) -> Vec<HelpEntry> {
        if cx.is_active_view(DIFF) {
            help(self)
        } else {
            Vec::new()
        }
    }

    fn hints(&self, target: HintTarget, cx: &Cx) -> Option<&'static str> {
        if !cx.is_active_view(DIFF) {
            return None;
        }
        match target {
            HintTarget::Focus(focus) if focus == DIFF_BAND_FOCUS => {
                Some("j/k choose commit   Tab files   Ctrl-P commands   ? help ")
            }
            HintTarget::Focus(focus) if focus == DIFF_BODY_FOCUS => {
                Some("wheel / j k scroll   / search   n/N match   Ctrl-P commands   ? help ")
            }
            HintTarget::Focus(_) | HintTarget::Overlay(_) => None,
        }
    }

    fn chips(&self, cx: &Cx) -> Vec<Chip> {
        if !cx.is_active_view(DIFF) {
            return Vec::new();
        }
        let (base, target) = self.revisions();
        vec![Chip {
            label: format!("{base}..{target}"),
            tone: ChipTone::Neutral,
        }]
    }

    fn status(&self, cx: &Cx) -> Vec<Segment> {
        if !cx.is_active_view(DIFF) {
            return Vec::new();
        }
        let (added, removed) = self.diff_stats();
        vec![
            Segment {
                text: format!("+{added} "),
                tone: SegmentTone::Added,
            },
            Segment {
                text: format!("−{removed} "),
                tone: SegmentTone::Removed,
            },
        ]
    }

    fn busy(&self) -> bool {
        self.loading
    }

    fn render(
        &self,
        pane: PaneId,
        geometry: PaneGeometry,
        focused: bool,
        cx: &Cx,
        frame: &mut Frame,
    ) {
        if !cx.is_active_view(DIFF) {
            return;
        }
        match pane {
            OLD_PANE => pane::render(
                self,
                cx,
                frame,
                geometry.outer,
                Side::Old,
                focused,
                geometry.edge,
            ),
            NEW_PANE => pane::render(
                self,
                cx,
                frame,
                geometry.outer,
                Side::New,
                focused,
                geometry.edge,
            ),
            COMMITS_PANE => {
                if let DiffViewState::Commits(commits) = &self.view {
                    commit_picker::render(
                        &commits.picker,
                        cx.theme,
                        cx.busy,
                        frame,
                        geometry.outer,
                        focused,
                        geometry.edge,
                    );
                }
            }
            _ => {}
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Cmd> {
        if !cx.is_active_view(DIFF)
            || (pane != OLD_PANE && pane != NEW_PANE && pane != COMMITS_PANE)
        {
            return Reply::unhandled();
        }
        match input {
            Input::Key(key) => self.key_input(pane, key),
            Input::Nav(nav) => {
                self.nav_input(pane, nav, cx);
                Reply::handled(Vec::new())
            }
            Input::Mouse(event) => {
                self.mouse_input(pane, event);
                Reply::handled(Vec::new())
            }
        }
    }

    fn run(&mut self, command: CommandId, cx: &Cx) -> Vec<Out<Cmd>> {
        if command.owner != DIFF || !cx.is_active_view(DIFF) {
            return Vec::new();
        }
        match command.name {
            "base.edit" => vec![Out::Kernel(Request::OpenPrompt {
                label: "base",
                initial: self.base.clone(),
                tag: BASE_PROMPT_TAG,
            })],
            "target.edit" => vec![Out::Kernel(Request::OpenPrompt {
                label: "target",
                initial: self.target.clone(),
                tag: TARGET_PROMPT_TAG,
            })],
            "view.commits" => self.switch_view(DiffView::Commits, cx),
            "view.range" => self.switch_view(DiffView::Range, cx),
            _ => Vec::new(),
        }
    }

    fn update(&mut self, msg: Msg, cx: &Cx) -> Vec<Out<Cmd>> {
        match msg {
            Msg::DiffLoaded { request, result } => self.loaded(request, result),
            Msg::HistoryLoaded { request, result } => self.history_loaded(request, result, cx),
            Msg::StepLoaded { index, result } => self.step_loaded(index, result, cx),
        }
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> Vec<Out<Cmd>> {
        if !cx.is_active_view(DIFF) {
            return Vec::new();
        }
        match event {
            Event::SelectionChanged => {
                self.set_body_scroll(0);
                Vec::new()
            }
            Event::Resized => {
                let max = self.rows().len().saturating_sub(1) as u16;
                self.set_body_scroll(self.body_scroll().min(max));
                Vec::new()
            }
            Event::RevealMatch(matched) => {
                let index = self.visual_row_for(matched.pane, matched.line).unwrap_or(0);
                if let Some(next) = tui_core::nav::scroll_offset_to(
                    index,
                    self.body_scroll() as usize,
                    cx.size.height.saturating_sub(4) as usize,
                ) {
                    self.set_body_scroll(next);
                }
                Vec::new()
            }
            Event::Requery(change) => {
                self.loading = true;
                let request = match change {
                    QueryChange::Mode(mode) => {
                        let mut request = self.request(cx.workspace.query.scope.selection.clone());
                        request.mode = *mode;
                        request
                    }
                    QueryChange::Scope(selection) => self.request(selection.clone()),
                };
                vec![Out::Effect(match &self.view {
                    DiffViewState::Range(_) => Cmd::Range(request),
                    DiffViewState::Commits(_) => Cmd::History(request),
                })]
            }
            Event::PromptSubmitted { tag, value } => self.prompt_submitted(*tag, value.clone(), cx),
            _ => Vec::new(),
        }
    }

    fn search(&mut self, needle: &str, cx: &Cx) -> Vec<SearchMatch> {
        if !cx.is_active_view(DIFF) {
            return Vec::new();
        }
        self.search_matches(needle, cx.workspace)
    }

    fn after_batch(&mut self, pending: &[Cmd], cx: &Cx) -> Vec<Out<Cmd>> {
        if !self.commits() {
            return Vec::new();
        }
        if pending
            .iter()
            .any(|cmd| matches!(cmd, Cmd::Range(_) | Cmd::History(_)))
        {
            return Vec::new();
        }
        let pending_step = if let DiffViewState::Commits(commits) = &self.view {
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
        let request = self.request(cx.workspace.query.scope.selection.clone());
        self.loading = true;
        vec![Out::Effect(Cmd::Step {
            request,
            index,
            step,
        })]
    }

    fn settle(&mut self, cx: &Cx) {
        if !cx.is_active_view(DIFF) {
            return;
        }
        let selected = cx.workspace.selected.clone();
        if cx.body_visible() {
            self.ensure_highlight(cx.theme, selected.as_ref());
        }
        self.refresh_rows(cx);
    }

    fn perform(engine: &Engine, cmd: Cmd) -> Msg {
        match cmd {
            Cmd::Range(request) => {
                let result = diff_result(engine, &request);
                Msg::DiffLoaded { request, result }
            }
            Cmd::History(request) => {
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
        }
    }

    fn startup_failure(msg: Msg) -> Result<Msg, Box<EngineError>> {
        match msg {
            Msg::DiffLoaded {
                result: Err(error), ..
            }
            | Msg::HistoryLoaded {
                result: Err(error), ..
            } => Err(error),
            other => Ok(other),
        }
    }
}

fn diff_result(
    engine: &Engine,
    request: &DiffRequest,
) -> Result<Arc<[FileDiff]>, Box<EngineError>> {
    let base = if request.merge_base {
        engine::DiffBase::MergeBase
    } else {
        engine::DiffBase::Given
    };
    engine
        .diff(
            &request.base,
            &request.target,
            request.mode,
            &request.selection,
            base,
        )
        .map(Arc::from)
        .map_err(Box::new)
}

impl DiffPlugin {
    fn key_input(&mut self, pane: PaneId, key: Key) -> Reply<Cmd> {
        if !matches!(key, Key::Down | Key::Char('j') | Key::Up | Key::Char('k')) {
            return Reply::unhandled();
        }
        if pane == COMMITS_PANE {
            if let DiffViewState::Commits(commits) = &mut self.view {
                commits.picker.update(commit_key(key));
            }
        } else {
            let max = self.rows().len().saturating_sub(1) as u16;
            let scroll = match key {
                Key::Down | Key::Char('j') => self.body_scroll().saturating_add(1).min(max),
                Key::Up | Key::Char('k') => self.body_scroll().saturating_sub(1),
                _ => self.body_scroll(),
            };
            self.set_body_scroll(scroll);
        }
        Reply::handled(Vec::new())
    }

    fn nav_input(&mut self, pane: PaneId, nav: Nav, cx: &Cx) {
        let step = page_step(cx.size.height);
        if pane == COMMITS_PANE {
            if let DiffViewState::Commits(commits) = &mut self.view {
                commits.picker.update(commit_move_msg(nav, step));
            }
            return;
        }
        let max = self.rows().len().saturating_sub(1) as u16;
        let scroll = scroll_by(nav, self.body_scroll(), step, max);
        self.set_body_scroll(scroll);
    }

    fn mouse_input(&mut self, pane: PaneId, event: MouseInput) {
        let inner = event.geometry.inner;
        let point = ratatui::layout::Position {
            x: event.column,
            y: event.row,
        };
        if pane == COMMITS_PANE {
            match event.kind {
                MouseKind::Click => {
                    if !inner.contains(point) {
                        return;
                    }
                    if let Some(index) = self.commit_click_index(inner, event.row)
                        && let DiffViewState::Commits(commits) = &mut self.view
                    {
                        commits.picker.update(commit_picker::Msg::Select(index));
                    }
                }
                MouseKind::ScrollUp => {
                    if let DiffViewState::Commits(commits) = &mut self.view {
                        commits
                            .picker
                            .update(commit_picker::Msg::Move(-MOUSE_SCROLL_STEP));
                    }
                }
                MouseKind::ScrollDown => {
                    if let DiffViewState::Commits(commits) = &mut self.view {
                        commits
                            .picker
                            .update(commit_picker::Msg::Move(MOUSE_SCROLL_STEP));
                    }
                }
            }
            return;
        }
        let delta = match event.kind {
            MouseKind::ScrollUp => -MOUSE_SCROLL_STEP,
            MouseKind::ScrollDown => MOUSE_SCROLL_STEP,
            MouseKind::Click => 0,
        };
        if delta != 0 {
            let max = self.rows().len().saturating_sub(1) as u16;
            let scroll = self.body_scroll();
            let next = if delta < 0 {
                scroll.saturating_sub(delta.unsigned_abs() as u16)
            } else {
                scroll.saturating_add(delta as u16).min(max)
            };
            self.set_body_scroll(next);
        }
    }

    fn loaded(
        &mut self,
        request: DiffRequest,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    ) -> Vec<Out<Cmd>> {
        match result {
            Ok(diffs) => {
                let was_commits = self.commits();
                self.loading = false;
                self.base = request.base;
                self.target = request.target;
                self.loaded_mode = request.mode;
                self.merge_base = request.merge_base;
                if !self.range() {
                    self.view = DiffViewState::Range(Range::empty());
                }
                if let DiffViewState::Range(range) = &mut self.view {
                    range.diffs = diffs;
                    range.highlight.clear();
                }
                self.set_body_scroll(0);
                self.invalidate();
                let file_set = file_set(self.diffs(), NO_CHANGES);
                let mut outs = Vec::new();
                if was_commits {
                    outs.push(Out::Kernel(Request::FocusDefault));
                }
                outs.push(Out::Kernel(Request::QueryLoaded(Query {
                    mode: request.mode,
                    scope: Scope::new(request.selection),
                })));
                outs.push(Out::Kernel(Request::SetFiles(file_set)));
                outs.push(Out::Kernel(Request::ClearDiagnostic));
                outs.push(Out::Kernel(Request::CancelPrefix));
                outs
            }
            Err(error) => {
                self.loading = false;
                vec![Out::Kernel(Request::Diagnose(error.to_string()))]
            }
        }
    }

    fn history_loaded(
        &mut self,
        request: DiffRequest,
        result: Result<HistoryPayload, Box<EngineError>>,
        _cx: &Cx,
    ) -> Vec<Out<Cmd>> {
        let (steps, diffs) = match result {
            Ok(payload) => payload,
            Err(error) => {
                self.loading = false;
                return vec![Out::Kernel(Request::Diagnose(error.to_string()))];
            }
        };
        self.loading = false;
        self.base = request.base;
        self.target = request.target;
        self.loaded_mode = request.mode;
        self.merge_base = request.merge_base;
        let selection = request.selection;
        let preferred = match &self.view {
            DiffViewState::Commits(commits) => commits
                .picker
                .steps_slice()
                .get(commits.picker.cursor())
                .map(|step| step.commit_id.clone()),
            DiffViewState::Range(_) => None,
        };
        self.view = DiffViewState::Commits(Commits {
            picker: CommitPicker::steps(steps),
            body: DiffBody { scroll: 0 },
            diffs,
            highlight: Default::default(),
        });
        self.invalidate();

        let mut outs = vec![
            Out::Kernel(Request::Focus(DIFF_BAND_FOCUS)),
            Out::Kernel(Request::QueryLoaded(Query {
                mode: request.mode,
                scope: Scope::new(selection.clone()),
            })),
            Out::Kernel(Request::SetFiles(file_set(self.diffs(), NO_CHANGES))),
            Out::Kernel(Request::ClearDiagnostic),
        ];

        let pending = if let DiffViewState::Commits(commits) = &self.view {
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
            self.loading = true;
            outs.push(Out::Effect(Cmd::Step {
                request: self.request(selection),
                index,
                step,
            }));
        }
        outs
    }

    fn step_loaded(
        &mut self,
        index: usize,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
        cx: &Cx,
    ) -> Vec<Out<Cmd>> {
        if !self.commits() {
            return Vec::new();
        }
        match result {
            Ok(diffs) => {
                let len = match &self.view {
                    DiffViewState::Commits(commits) => commits.picker.steps_slice().len(),
                    DiffViewState::Range(_) => 0,
                };
                if index < len {
                    self.loading = false;
                    let inner_height = self
                        .commits_inner(cx)
                        .map_or(0, |rect| rect.height as usize);
                    if let DiffViewState::Commits(commits) = &mut self.view {
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
                    self.set_body_scroll(0);
                    self.invalidate();
                    let file_set = file_set(self.diffs(), NO_CHANGES);
                    vec![
                        Out::Kernel(Request::SetFiles(file_set)),
                        Out::Kernel(Request::ClearDiagnostic),
                    ]
                } else {
                    if let DiffViewState::Commits(commits) = &mut self.view {
                        commits.picker.update(commit_picker::Msg::Failed);
                    }
                    Vec::new()
                }
            }
            Err(error) => {
                self.loading = false;
                if let DiffViewState::Commits(commits) = &mut self.view {
                    commits.picker.update(commit_picker::Msg::Failed);
                }
                vec![Out::Kernel(Request::Diagnose(error.to_string()))]
            }
        }
    }
}

fn commit_key(key: Key) -> commit_picker::Msg {
    match key {
        Key::Down | Key::Char('j') => commit_picker::Msg::Move(1),
        Key::Up | Key::Char('k') => commit_picker::Msg::Move(-1),
        _ => commit_picker::Msg::Move(0),
    }
}

fn commit_move_msg(nav: Nav, step: u16) -> commit_picker::Msg {
    match nav {
        Nav::Top => commit_picker::Msg::ToTop,
        Nav::Bottom => commit_picker::Msg::ToBottom,
        Nav::PageUp => commit_picker::Msg::Page(-i32::from(step)),
        Nav::PageDown => commit_picker::Msg::Page(i32::from(step)),
    }
}

/// The workspace file set for a diff load: the paths and their badge tones.
fn file_set(diffs: &[FileDiff], empty: EmptyText) -> FileSet {
    let paths: Arc<[RepoPath]> = Arc::from(diff_visible(diffs));
    let badges: BTreeMap<RepoPath, BadgeTone> = diffs
        .iter()
        .map(|diff| {
            let tone = match diff {
                FileDiff::Added { .. } => BadgeTone::Added,
                FileDiff::Deleted { .. } => BadgeTone::Deleted,
                FileDiff::Modified { .. } => BadgeTone::Modified,
            };
            (diff.path().clone(), tone)
        })
        .collect();
    FileSet {
        paths,
        badges: Arc::new(badges),
        empty,
    }
}
