//! The app shell: the one place that owns shared state (the terminal size, the
//! tree, the chrome, the overlay, the diagnostic), routes input to the page or
//! component that owns it, and folds their `OutMsg`s back into shared state and
//! effects.
//!
//! A page owns its own model and `update`; the shell routes to it and never
//! reaches into its logic. Nothing here performs I/O: `update` turns a message
//! and the current model into a replacement model plus a list of effects to
//! run. Engine calls only ever leave this module as a [`Cmd`]. The pure `view`
//! lives in [`crate::view`].

use std::sync::Arc;

use base::{AreaSet, FileDiff, ProjectedFile, ProjectionMode, RepoPath, Selection};
use engine::EngineError;
use ratatui::layout::{Position, Rect};

use crate::action::{Action, CommitsAction, GlobalAction, RangeAction, ShowAction};
pub use crate::api::{Cmd, HistoryPayload};
use crate::component::commit_picker;
use crate::component::overlay::{self, Overlay};
use crate::component::tree::{self, RowKind, Tree};
use crate::page::{
    Commits, Ctx, Diff, DiffSide, DiffViewState, Loaded, OutMsg, Paging, Range, Scope, Show,
    ShowBody, ShowFocus, diff as diff_page, show as show_page,
};
use crate::render::block::pane_block;
use crate::render::icons::Icons;
use crate::render::layout::{PaneSlot, body_layout, commit_offset, frame_areas, window_offset};
use crate::render::metrics::{Focus, MIN_HEIGHT, SIDE_BY_SIDE_MIN_WIDTH, SINGLE_PANE_MIN_WIDTH};
use crate::render::theme::Theme;
pub use crate::route::{DiffRequest, DiffView, LoadRequest, ShowRequest};
use crate::util::input::{Key, Mouse, MouseKind};

/// The file-tree pane width, as a percentage of the terminal, and its bounds.
const TREE_DEFAULT_PERCENT: u16 = 30;
const TREE_MIN_PERCENT: u16 = 15;
const TREE_MAX_PERCENT: u16 = 60;
const TREE_STEP: u16 = 5;

/// How many ticks a diagnostic stays visible before it fades out.
const DIAGNOSTIC_TICKS: u8 = 40;

/// How many rows one wheel notch moves.
const MOUSE_SCROLL_STEP: i32 = 3;

/// The terminal size, always set as a pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

/// State that outlives any projection.
#[derive(Clone)]
pub struct Chrome {
    /// The file-tree pane width as a percentage of the terminal.
    pub tree_percent: u16,
    /// The resolved design tokens; view code never names a raw color.
    pub theme: Theme,
    /// The resolved tree glyph set; icons are opt-in.
    pub icons: Icons,
    /// The spinner animation frame, advanced by [`Msg::Tick`].
    pub spinner: u8,
    pub quit: bool,
}

/// A self-expiring status message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub text: String,
    pub ttl: u8,
}

impl Diagnostic {
    fn new(text: String) -> Self {
        Self {
            text,
            ttl: DIAGNOSTIC_TICKS,
        }
    }
}

/// Every input the core accepts.
#[derive(Debug)]
pub enum Msg {
    Key(Key),
    Mouse(Mouse),
    Resize {
        width: u16,
        height: u16,
    },
    /// A periodic tick, used to animate the spinner and expire a diagnostic.
    Tick,
    /// A `show` projection effect finished.
    ShowLoaded {
        request: ShowRequest,
        result: Result<Arc<[ProjectedFile]>, Box<EngineError>>,
    },
    /// A `diff` range effect finished.
    DiffLoaded {
        request: DiffRequest,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
    /// A commits-view effect finished: the steps and the first step's diff.
    HistoryLoaded {
        request: DiffRequest,
        result: Result<HistoryPayload, Box<EngineError>>,
    },
    /// A single commit step's projection finished.
    StepLoaded {
        index: usize,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
    /// The area configuration finished loading for the scope chooser.
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

/// The entire UI state. It is replaced wholesale by `update`, never edited in
/// place by anything else.
#[derive(Clone)]
pub struct App {
    pub root: String,
    pub size: Size,
    pub chrome: Chrome,
    pub tree: Tree,
    pub overlay: Option<Overlay>,
    pub loaded: Loaded,
    pub diagnostic: Option<Diagnostic>,
}

impl App {
    /// A model whose projection is still loading.
    pub fn new(
        root: String,
        request: LoadRequest,
        scope_label: String,
        width: u16,
        height: u16,
        theme: Theme,
        icons: Icons,
    ) -> Self {
        let loaded = match request {
            LoadRequest::Show {
                revision,
                mode,
                selection,
            } => Loaded::Show(Show {
                revision,
                mode,
                scope: Scope {
                    selection,
                    label: scope_label,
                },
                files: Arc::from(Vec::new()),
                visible: Arc::from(Vec::new()),
                selected: None,
                focus: ShowFocus::Tree,
                body: ShowBody {
                    scroll: 0,
                    hscroll: 0,
                },
                paging: Paging::Loading,
                search: None,
                highlight: Default::default(),
                prompt: None,
            }),
            LoadRequest::Diff {
                base,
                target,
                mode,
                selection,
                view,
                merge_base,
            } => Loaded::Diff(Diff::new(
                base,
                target,
                mode,
                Scope {
                    selection,
                    label: scope_label,
                },
                match view {
                    DiffView::Range => DiffViewState::Range(Range::empty()),
                    DiffView::Commits => DiffViewState::Commits(Commits::empty()),
                },
                merge_base,
            )),
        };
        Self {
            root,
            size: Size { width, height },
            chrome: Chrome {
                tree_percent: TREE_DEFAULT_PERCENT,
                theme,
                icons,
                spinner: 0,
                quit: false,
            },
            tree: Tree::default(),
            overlay: None,
            loaded,
            diagnostic: None,
        }
    }

    /// Whether a projection is in flight, so the view can spin.
    pub fn is_busy(&self) -> bool {
        self.loaded.is_loading()
    }

    pub(crate) fn diff_rows(&self) -> &[crate::render::layout::VisualRow] {
        self.loaded.diff_rows()
    }

    /// The wrapped rows of the active diff, recomputed without the cache.
    #[cfg(feature = "bench")]
    pub(crate) fn compute_diff_rows(&self) -> Vec<crate::render::layout::VisualRow> {
        match &self.loaded {
            Loaded::Diff(diff) => diff.compute_rows(
                self.size.width,
                self.size.height,
                self.chrome.tree_percent,
                &self.chrome.theme,
            ),
            Loaded::Show(_) => Vec::new(),
        }
    }

    pub(crate) fn scope_label(&self) -> &str {
        &self.loaded.scope().label
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// The pure update function. It performs no I/O and reads no external state.
pub fn update(msg: Msg, app: &App) -> (App, Vec<Cmd>) {
    let mut next = app.clone();
    let mut cmds = Vec::new();
    let ctx = ctx(&next);

    match msg {
        Msg::Key(key) => handle_key(&mut next, key, &mut cmds),
        Msg::Mouse(event) => mouse(&mut next, event, &mut cmds),
        Msg::Resize { width, height } => {
            next.size = Size { width, height };
            next.loaded.clamp_scroll();
        }
        Msg::Tick => {
            next.chrome.spinner = next.chrome.spinner.wrapping_add(1);
            if let Some(diagnostic) = &mut next.diagnostic {
                diagnostic.ttl = diagnostic.ttl.saturating_sub(1);
                if diagnostic.ttl == 0 {
                    next.diagnostic = None;
                }
            }
        }
        Msg::ShowLoaded { request, result } => {
            let installed = result.is_ok();
            let outs = route_show(&mut next, show_page::Msg::Loaded(request, result), ctx);
            fold(&mut next, outs, &mut cmds);
            if installed {
                install_show_selection(&mut next);
                next.diagnostic = None;
            }
        }
        Msg::DiffLoaded { request, result } => {
            let installed = result.is_ok();
            let outs = route_diff(&mut next, diff_page::Msg::Loaded(request, result), ctx);
            fold(&mut next, outs, &mut cmds);
            if installed {
                install_diff_selection(&mut next);
                next.diagnostic = None;
            }
        }
        Msg::HistoryLoaded { request, result } => {
            let installed = result.is_ok();
            let outs = route_diff(
                &mut next,
                diff_page::Msg::HistoryLoaded(request, result),
                ctx,
            );
            fold(&mut next, outs, &mut cmds);
            if installed {
                install_diff_selection(&mut next);
                next.diagnostic = None;
            }
        }
        Msg::StepLoaded { index, result } => {
            let inner_height = commits_inner(&next).map_or(0, |rect| rect.height as usize);
            let installed = result.is_ok() && step_in_range(&next, index);
            let outs = route_diff(
                &mut next,
                diff_page::Msg::StepLoaded {
                    index,
                    inner_height,
                    result,
                },
                ctx,
            );
            fold(&mut next, outs, &mut cmds);
            if installed {
                install_diff_selection(&mut next);
                next.diagnostic = None;
            }
        }
        Msg::AreasLoaded(result) => {
            let outs = overlay_update(&mut next, overlay::Msg::AreasLoaded(result));
            for out in outs {
                apply_overlay_out(&mut next, out, &mut cmds);
            }
        }
    }

    (next, cmds)
}

fn ctx(app: &App) -> Ctx {
    Ctx {
        height: app.size.height,
    }
}

/// Folds a page's requests into shared state and the command list.
fn fold(app: &mut App, outs: Vec<OutMsg>, cmds: &mut Vec<Cmd>) {
    for out in outs {
        match out {
            OutMsg::Effect(cmd) => cmds.push(cmd),
            OutMsg::Diagnose(text) => app.diagnostic = Some(Diagnostic::new(text)),
        }
    }
}

fn route_show(app: &mut App, msg: show_page::Msg, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(msg, show, &ctx),
        Loaded::Diff(_) => Vec::new(),
    }
}

fn route_diff(app: &mut App, msg: diff_page::Msg, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Diff(diff) => diff_page::update(msg, diff, &ctx),
        Loaded::Show(_) => Vec::new(),
    }
}

fn route_page_key(app: &mut App, key: Key, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::Key(key), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::Key(key), diff, &ctx),
    }
}

fn route_page_action(app: &mut App, action: GlobalAction, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::Action(action), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::Action(action), diff, &ctx),
    }
}

fn route_cycle_focus(app: &mut App, forward: bool, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::CycleFocus(forward), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::CycleFocus(forward), diff, &ctx),
    }
}

fn route_step_search(app: &mut App, forward: bool, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::StepSearch(forward), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::StepSearch(forward), diff, &ctx),
    }
}

fn route_scroll_current(app: &mut App, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::ScrollToCurrent, show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::ScrollToCurrent, diff, &ctx),
    }
}

fn route_set_search(app: &mut App, needle: &str, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => {
            show_page::update(show_page::Msg::SetSearch(needle.to_owned()), show, &ctx)
        }
        Loaded::Diff(diff) => {
            diff_page::update(diff_page::Msg::SetSearch(needle.to_owned()), diff, &ctx)
        }
    }
}

fn route_clear_search(app: &mut App, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::ClearSearch, show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::ClearSearch, diff, &ctx),
    }
}

fn route_reload_scope(app: &mut App, selection: Selection, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::ReloadScope(selection), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::ReloadScope(selection), diff, &ctx),
    }
}

fn route_switch_mode(app: &mut App, mode: ProjectionMode, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::SwitchMode(mode), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::SwitchMode(mode), diff, &ctx),
    }
}

fn route_prompt(app: &mut App, key: Key, ctx: Ctx) -> Vec<OutMsg> {
    match &mut app.loaded {
        Loaded::Show(show) => show_page::update(show_page::Msg::Prompt(key), show, &ctx),
        Loaded::Diff(diff) => diff_page::update(diff_page::Msg::Prompt(key), diff, &ctx),
    }
}

/// Sets the selection after a content install: keep the previous path when it
/// is still visible, otherwise choose the nearest visible file.
fn install_show_selection(app: &mut App) {
    let Loaded::Show(show) = &mut app.loaded else {
        return;
    };
    show.paging = Paging::Ready;
    let previous = show.selected.clone();
    let hint = app.tree.cursor;
    app.tree.sync(&show.visible);
    let target = previous
        .filter(|path| show.visible.contains(path))
        .or_else(|| nearest_visible(&app.tree, hint));
    match target {
        Some(path) => {
            show.selected = Some(path.clone());
            app.tree.reveal(&path);
        }
        None => {
            show.selected = None;
            if let Some(index) = app.tree.first_file_row() {
                app.tree.cursor = index;
            }
        }
    }
    show.body = ShowBody {
        scroll: 0,
        hscroll: 0,
    };
}

fn install_diff_selection(app: &mut App) {
    let Loaded::Diff(diff) = &mut app.loaded else {
        return;
    };
    let previous = diff.selected.clone();
    let hint = app.tree.cursor;
    app.tree.sync(&diff.visible);
    let target = previous
        .filter(|path| diff.visible.contains(path))
        .or_else(|| nearest_visible(&app.tree, hint));
    match target {
        Some(path) => {
            diff.selected = Some(path.clone());
            app.tree.reveal(&path);
        }
        None => {
            diff.selected = None;
            if let Some(index) = app.tree.first_file_row() {
                app.tree.cursor = index;
            }
        }
    }
    diff.set_body_scroll(0);
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

// ---------------------------------------------------------------------------
// Key routing
// ---------------------------------------------------------------------------

fn handle_key(app: &mut App, key: Key, cmds: &mut Vec<Cmd>) {
    if key == Key::CtrlC {
        app.chrome.quit = true;
        return;
    }
    let capturing =
        app.overlay.as_ref().is_some_and(Overlay::captures_text) || app.loaded.prompt_active();
    if key == Key::Char('q') && !capturing {
        app.chrome.quit = true;
        return;
    }
    if app.overlay.is_some() {
        let outs = overlay_update(app, overlay::Msg::Key(key));
        for out in outs {
            apply_overlay_out(app, out, cmds);
        }
        return;
    }
    if app.loaded.prompt_active() {
        let ctx = ctx(app);
        let outs = route_prompt(app, key, ctx);
        fold(app, outs, cmds);
        return;
    }
    if let Some(action) = command_for(key, app) {
        apply_action(app, action, cmds);
        return;
    }
    match key {
        Key::Esc => {
            app.diagnostic = None;
            let ctx = ctx(app);
            let outs = route_clear_search(app, ctx);
            fold(app, outs, cmds);
        }
        _ => focus_key(app, key, cmds),
    }
}

fn command_for(key: Key, app: &App) -> Option<Action> {
    use GlobalAction::*;
    let show = matches!(app.loaded, Loaded::Show(_));
    let diff = matches!(app.loaded, Loaded::Diff(_));
    let global = match key {
        Key::Char('?') => Help,
        Key::Char('s') => Scope,
        Key::Char('m') => Mode,
        Key::Char('[') => TreeNarrower,
        Key::Char(']') => TreeWider,
        Key::Char('\\') => TreeReset,
        Key::Tab => NextPane,
        Key::BackTab => PreviousPane,
        Key::Char('g') => Top,
        Key::Char('G') => Bottom,
        Key::Char('n') => NextMatch,
        Key::Char('N') => PreviousMatch,
        Key::CtrlP => Palette,
        Key::CtrlF => Finder,
        Key::Char('/') => Search,
        Key::PageDown | Key::CtrlD => PageDown,
        Key::PageUp | Key::CtrlU => PageUp,
        Key::Char('r') if show => {
            return Some(Action::Show(ShowAction::EditRevision));
        }
        Key::Char('b') if diff => {
            return Some(Action::Range(RangeAction::EditBase));
        }
        Key::Char('t') if diff => {
            return Some(Action::Range(RangeAction::EditTarget));
        }
        _ => return None,
    };
    Some(Action::Global(global))
}

fn apply_action(app: &mut App, action: Action, cmds: &mut Vec<Cmd>) {
    let ctx = ctx(app);
    match action {
        Action::Global(action) => apply_global(app, action, cmds),
        Action::Show(ShowAction::EditRevision) => {
            let outs = route_show(app, show_page::Msg::EditRevision, ctx);
            fold(app, outs, cmds);
        }
        Action::Range(RangeAction::EditBase) => {
            let outs = route_diff(app, diff_page::Msg::EditSide(DiffSide::Base), ctx);
            fold(app, outs, cmds);
        }
        Action::Range(RangeAction::EditTarget) => {
            let outs = route_diff(app, diff_page::Msg::EditSide(DiffSide::Target), ctx);
            fold(app, outs, cmds);
        }
        Action::Range(RangeAction::SwitchToCommits) => {
            let outs = route_diff(app, diff_page::Msg::SwitchView(DiffView::Commits), ctx);
            fold(app, outs, cmds);
        }
        Action::Commits(CommitsAction::SwitchToRange) => {
            let outs = route_diff(app, diff_page::Msg::SwitchView(DiffView::Range), ctx);
            fold(app, outs, cmds);
        }
    }
}

fn apply_global(app: &mut App, action: GlobalAction, cmds: &mut Vec<Cmd>) {
    use GlobalAction::*;
    let ctx = ctx(app);
    match action {
        Quit => app.chrome.quit = true,
        Help => app.overlay = Some(Overlay::help()),
        Scope => {
            app.overlay = Some(Overlay::scope());
            cmds.push(Cmd::LoadAreas);
        }
        Mode => app.overlay = Some(Overlay::mode(app.loaded.mode())),
        Palette => {
            let entries = app.loaded.palette_entries();
            let mode = app.loaded.mode();
            let visible = app.loaded.visible();
            app.overlay = Some(Overlay::palette(&overlay::Ctx {
                visible,
                entries: &entries,
                mode,
            }));
        }
        Finder => {
            let entries = app.loaded.palette_entries();
            let mode = app.loaded.mode();
            let visible = app.loaded.visible();
            app.overlay = Some(Overlay::finder(&overlay::Ctx {
                visible,
                entries: &entries,
                mode,
            }));
        }
        Search => app.overlay = Some(Overlay::search()),
        TreeWider => {
            app.chrome.tree_percent = (app.chrome.tree_percent + TREE_STEP).min(TREE_MAX_PERCENT);
        }
        TreeNarrower => {
            app.chrome.tree_percent = app
                .chrome
                .tree_percent
                .saturating_sub(TREE_STEP)
                .max(TREE_MIN_PERCENT);
        }
        TreeReset => app.chrome.tree_percent = TREE_DEFAULT_PERCENT,
        NextPane => {
            let outs = route_cycle_focus(app, true, ctx);
            fold(app, outs, cmds);
        }
        PreviousPane => {
            let outs = route_cycle_focus(app, false, ctx);
            fold(app, outs, cmds);
        }
        Top | Bottom | PageUp | PageDown => focus_command(app, action, cmds),
        NextMatch => {
            let outs = route_step_search(app, true, ctx);
            fold(app, outs, cmds);
        }
        PreviousMatch => {
            let outs = route_step_search(app, false, ctx);
            fold(app, outs, cmds);
        }
    }
}

fn focus_command(app: &mut App, action: GlobalAction, cmds: &mut Vec<Cmd>) {
    if app.loaded.focus() == Focus::Tree {
        let step = page_step(app.size.height);
        tree_apply(app, page_tree_msg(action, step));
        return;
    }
    let ctx = ctx(app);
    let outs = route_page_action(app, action, ctx);
    fold(app, outs, cmds);
}

fn focus_key(app: &mut App, key: Key, cmds: &mut Vec<Cmd>) {
    if app.loaded.focus() == Focus::Tree {
        if let Some(msg) = tree_key(key) {
            tree_apply(app, msg);
        }
        return;
    }
    let ctx = ctx(app);
    let outs = route_page_key(app, key, ctx);
    fold(app, outs, cmds);
}

fn page_step(height: u16) -> u16 {
    (height.saturating_sub(3) / 2).max(1)
}

fn page_tree_msg(action: GlobalAction, step: u16) -> tree::Msg {
    match action {
        GlobalAction::Top => tree::Msg::ToTop,
        GlobalAction::Bottom => tree::Msg::ToBottom,
        GlobalAction::PageUp => tree::Msg::Page(-i32::from(step)),
        _ => tree::Msg::Page(i32::from(step)),
    }
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

fn tree_apply(app: &mut App, msg: tree::Msg) {
    let selected = app.tree.update(msg, app.loaded.visible());
    if let Some(tree::OutMsg::Selected(path)) = selected {
        select_path(app, path);
    }
}

fn select_path(app: &mut App, path: RepoPath) {
    app.loaded.set_selected(Some(path.clone()));
    app.tree.reveal(&path);
    app.loaded.reset_body_scroll();
}

// ---------------------------------------------------------------------------
// Overlay
// ---------------------------------------------------------------------------

fn overlay_update(app: &mut App, msg: overlay::Msg) -> Vec<overlay::OutMsg> {
    let Some(mut current) = app.overlay.take() else {
        return Vec::new();
    };
    let entries = app.loaded.palette_entries();
    let mode = app.loaded.mode();
    let visible = app.loaded.visible();
    let ctx = overlay::Ctx {
        visible,
        entries: &entries,
        mode,
    };
    let outs = current.update(msg, &ctx);
    app.overlay = Some(current);
    outs
}

fn apply_overlay_out(app: &mut App, out: overlay::OutMsg, cmds: &mut Vec<Cmd>) {
    let ctx = ctx(app);
    match out {
        overlay::OutMsg::Close => app.overlay = None,
        overlay::OutMsg::Selection(selection) => {
            let outs = route_reload_scope(app, selection, ctx);
            fold(app, outs, cmds);
        }
        overlay::OutMsg::Mode(mode) => {
            let outs = route_switch_mode(app, mode, ctx);
            fold(app, outs, cmds);
        }
        overlay::OutMsg::Action(action) => apply_action(app, action, cmds),
        overlay::OutMsg::File(path) => select_path(app, path),
        overlay::OutMsg::Search(needle) => {
            let outs = route_set_search(app, &needle, ctx);
            fold(app, outs, cmds);
        }
        overlay::OutMsg::SearchCommit => {
            let outs = route_scroll_current(app, ctx);
            fold(app, outs, cmds);
        }
    }
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

fn mouse(app: &mut App, event: Mouse, cmds: &mut Vec<Cmd>) {
    if app.overlay.is_some()
        || app.size.width < SINGLE_PANE_MIN_WIDTH
        || app.size.height < MIN_HEIGHT
    {
        return;
    }

    let is_diff = matches!(app.loaded, Loaded::Diff(_));
    let has_commits =
        matches!(&app.loaded, Loaded::Diff(diff) if matches!(diff.view, DiffViewState::Commits(_)));
    let (_, content, _) = frame_areas(app.size.width, app.size.height);
    let layout = body_layout(
        content,
        app.chrome.tree_percent,
        is_diff,
        has_commits,
        app.loaded.focus(),
    );
    let point = Position {
        x: event.column,
        y: event.row,
    };
    let Some(slot) = layout.slots.iter().find(|slot| slot.outer.contains(point)) else {
        return;
    };
    let inner = pane_block("", false, &app.chrome.theme, slot.edge).inner(slot.outer);
    let ctx = ctx(app);

    match slot.pane {
        PaneSlot::Commits => {
            app.loaded.set_focus(Focus::Commits);
            let msg = match event.kind {
                MouseKind::Click => {
                    if !inner.contains(point) {
                        return;
                    }
                    commit_click_index(app, inner, event.row).map(commit_picker::Msg::Select)
                }
                MouseKind::ScrollUp => Some(commit_picker::Msg::Move(-MOUSE_SCROLL_STEP)),
                MouseKind::ScrollDown => Some(commit_picker::Msg::Move(MOUSE_SCROLL_STEP)),
            };
            if let Some(msg) = msg {
                let outs = route_diff(app, diff_page::Msg::Commit(msg), ctx);
                fold(app, outs, cmds);
            }
        }
        PaneSlot::Tree => {
            app.loaded.set_focus(Focus::Tree);
            match event.kind {
                MouseKind::Click => {
                    let inner_height = inner.height as usize;
                    let offset = window_offset(app.tree.cursor, app.tree.rows.len(), inner_height);
                    let index = offset + usize::from(event.row.saturating_sub(inner.y));
                    if index < app.tree.rows.len() {
                        tree_apply(app, tree::Msg::Click(index));
                    }
                }
                MouseKind::ScrollUp => tree_apply(app, tree::Msg::Move(-MOUSE_SCROLL_STEP)),
                MouseKind::ScrollDown => tree_apply(app, tree::Msg::Move(MOUSE_SCROLL_STEP)),
            }
        }
        PaneSlot::Show | PaneSlot::Old | PaneSlot::New => {
            app.loaded.set_focus(Focus::Content);
            let delta = match event.kind {
                MouseKind::ScrollUp => -MOUSE_SCROLL_STEP,
                MouseKind::ScrollDown => MOUSE_SCROLL_STEP,
                MouseKind::Click => 0,
            };
            if delta != 0 {
                let outs = match &mut app.loaded {
                    Loaded::Show(show) => {
                        show_page::update(show_page::Msg::Scroll(delta), show, &ctx)
                    }
                    Loaded::Diff(diff) => {
                        diff_page::update(diff_page::Msg::Scroll(delta), diff, &ctx)
                    }
                };
                fold(app, outs, cmds);
            }
        }
    }
}

fn commits_inner(app: &App) -> Option<Rect> {
    let (_, content, _) = frame_areas(app.size.width, app.size.height);
    let layout = body_layout(content, app.chrome.tree_percent, true, true, Focus::Commits);
    let slot = layout
        .slots
        .iter()
        .find(|slot| slot.pane == PaneSlot::Commits)?;
    Some(pane_block("", false, &app.chrome.theme, slot.edge).inner(slot.outer))
}

fn commit_click_index(app: &App, inner: Rect, row: u16) -> Option<usize> {
    let Loaded::Diff(diff) = &app.loaded else {
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

/// Whether a step index is within the current commit list.
fn step_in_range(app: &App, index: usize) -> bool {
    match &app.loaded {
        Loaded::Diff(diff) => match &diff.view {
            DiffViewState::Commits(commits) => index < commits.picker.steps_slice().len(),
            DiffViewState::Range(_) => false,
        },
        Loaded::Show(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Derived state
// ---------------------------------------------------------------------------

fn body_is_visible(app: &App) -> bool {
    if app.size.width < SINGLE_PANE_MIN_WIDTH || app.size.height < MIN_HEIGHT {
        return false;
    }
    app.size.width >= SIDE_BY_SIDE_MIN_WIDTH || app.loaded.focus() != Focus::Tree
}

/// The focus, normalized for layout.
pub(crate) fn layout_focus(app: &App) -> Focus {
    app.loaded.focus()
}

/// Runs the selection-dependent work a frame needs, once per input batch.
pub(crate) fn settle(mut app: App) -> App {
    let visible_body = body_is_visible(&app);
    let width = app.size.width;
    let height = app.size.height;
    let tree_percent = app.chrome.tree_percent;
    app.loaded
        .settle(&app.chrome.theme, visible_body, width, height, tree_percent);
    app
}

/// Replaces all step projections in one input batch with its final target. A
/// full reload wins when the view or endpoints also changed.
pub(crate) fn coalesce_commit_loads(app: &mut App, cmds: &mut Vec<Cmd>) {
    let Loaded::Diff(diff) = &mut app.loaded else {
        return;
    };
    if !matches!(diff.view, DiffViewState::Commits(_)) {
        return;
    }
    if cmds.iter().any(|cmd| matches!(cmd, Cmd::Diff(_))) {
        return;
    }
    let pending = if let DiffViewState::Commits(commits) = &diff.view {
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
    if let Some((index, step)) = pending {
        let request = diff.request();
        diff.paging = Paging::Loading;
        cmds.push(Cmd::Step {
            request,
            index,
            step,
        });
    }
}

#[cfg(test)]
mod tests;
