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
//!
//! A fold prefix (`z`) opens the which-key band; the next key runs a binding,
//! and `Esc`, a mouse event, another overlay, or a reload closes it.

use std::sync::Arc;

use base::{AreaSet, FileDiff, ProjectionMode, RepoPath, Selection, SourceOutline};
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
use crate::render::layout::{PaneSlot, body_layout, commit_offset, frame_areas};
use crate::render::metrics::{Focus, MIN_HEIGHT, SIDE_BY_SIDE_MIN_WIDTH, SINGLE_PANE_MIN_WIDTH};
use crate::render::theme::Theme;
use crate::request::{NavigatorResize, QueryChange, Request};
pub use crate::route::{DiffRequest, DiffView, LoadRequest, ShowRequest};
use crate::util::input::{Key, Mouse, MouseKind};

const TREE_DEFAULT_PERCENT: u16 = 30;
const TREE_MIN_PERCENT: u16 = 15;
const TREE_MAX_PERCENT: u16 = 60;
const TREE_STEP: u16 = 5;

const DIAGNOSTIC_TICKS: u8 = 40;

const MOUSE_SCROLL_STEP: i32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

#[derive(Clone)]
pub struct Chrome {
    pub tree_percent: u16,
    pub theme: Theme,
    pub icons: Icons,
    pub spinner: u8,
    pub quit: bool,
}

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

/// The pending fold prefix, owned by the shell rather than the which-key
/// overlay. `pending` is `Some(c)` exactly while a `Prefixed(c, _)` binding is
/// armed and the which-key band is open.
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    pub pending: Option<char>,
}

#[derive(Debug)]
pub enum Msg {
    Key(Key),
    Mouse(Mouse),
    Resize {
        width: u16,
        height: u16,
    },
    Tick,
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
        result: Result<HistoryPayload, Box<EngineError>>,
    },
    StepLoaded {
        index: usize,
        result: Result<Arc<[FileDiff]>, Box<EngineError>>,
    },
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

#[derive(Clone)]
pub struct App {
    pub root: String,
    pub size: Size,
    pub chrome: Chrome,
    pub tree: Tree,
    pub overlay: Option<Overlay>,
    pub keymap: Keymap,
    pub loaded: Loaded,
    pub diagnostic: Option<Diagnostic>,
}

impl App {
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
                    cursor_line: 1,
                },
                paging: Paging::Loading,
                search: None,
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
            keymap: Keymap::default(),
            loaded,
            diagnostic: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.loaded.is_loading()
    }

    pub(crate) fn diff_rows(&self) -> &[crate::render::layout::VisualRow] {
        self.loaded.diff_rows()
    }

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

pub fn update(msg: Msg, app: &App) -> (App, Vec<Cmd>) {
    let mut next = app.clone();
    let mut cmds = Vec::new();
    let ctx = ctx(&next);

    match msg {
        Msg::Key(key) => handle_key(&mut next, key, &mut cmds),
        Msg::Mouse(event) => {
            clear_prefix(&mut next);
            mouse(&mut next, event, &mut cmds);
        }
        Msg::Resize { width, height } => {
            next.size = Size { width, height };
            next.loaded.clamp_scroll(height);
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
                clear_prefix(&mut next);
            }
        }
        Msg::DiffLoaded { request, result } => {
            let installed = result.is_ok();
            let outs = route_diff(&mut next, diff_page::Msg::Loaded(request, result), ctx);
            fold(&mut next, outs, &mut cmds);
            if installed {
                install_diff_selection(&mut next);
                next.diagnostic = None;
                clear_prefix(&mut next);
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

pub(crate) fn fold(app: &mut App, outs: Vec<OutMsg>, cmds: &mut Vec<Cmd>) {
    for out in outs {
        match out {
            OutMsg::Effect(cmd) => cmds.push(cmd),
            OutMsg::Request(request) => apply_request(app, request, cmds),
        }
    }
}

/// The depth a request cascade may reach before the kernel gives up. The plan's
/// section 4.7 fixes this at 16.
const REQUEST_DEPTH_LIMIT: u8 = 16;

/// Apply a kernel request and any requests its broadcast handlers produce.
///
/// Requests nest depth-first: a request produced while applying another is
/// applied before that request's next sibling. The depth limit guards against
/// a cycle; exceeding it is a bug in a debug build.
pub(crate) fn apply_request(app: &mut App, request: Request, cmds: &mut Vec<Cmd>) {
    apply_at(app, request, cmds, 0);
}

fn apply_at(app: &mut App, request: Request, cmds: &mut Vec<Cmd>, depth: u8) {
    if depth >= REQUEST_DEPTH_LIMIT {
        debug_assert!(false, "request cascade exceeded depth");
        app.diagnostic = Some(Diagnostic::new("request depth exceeded".to_owned()));
        return;
    }
    let nested = apply_one(app, request, cmds);
    for next in nested {
        apply_at(app, next, cmds, depth + 1);
    }
}

/// Perform one request's kernel effect and return the requests its broadcast
/// handlers produced, in broadcast order.
fn apply_one(app: &mut App, request: Request, cmds: &mut Vec<Cmd>) -> Vec<Request> {
    let mut nested = Vec::new();
    match request {
        Request::Quit => app.chrome.quit = true,
        Request::Diagnose(text) => app.diagnostic = Some(Diagnostic::new(text)),
        Request::ClearDiagnostic => app.diagnostic = None,
        Request::SetFiles(set) => {
            if let Some(selected) = app.loaded.selected()
                && !set.paths.iter().any(|path| path == selected)
            {
                app.loaded.set_selected(None);
            }
            on_files_changed(app);
        }
        Request::Select(selection) => {
            select(app, selection);
            on_selection_changed(app);
        }
        Request::Requery(change) => {
            let ctx = ctx(app);
            let outs = match change {
                QueryChange::Mode(mode) => route_switch_mode(app, mode, ctx),
                QueryChange::Scope(selection) => route_reload_scope(app, selection, ctx),
            };
            split_outs(outs, cmds, &mut nested);
        }
        Request::QueryLoaded(_query) => on_query_changed(app),
        Request::SetSearch(needle) => {
            let ctx = ctx(app);
            let outs = if needle.is_empty() {
                route_clear_search(app, ctx)
            } else {
                route_set_search(app, &needle, ctx)
            };
            split_outs(outs, cmds, &mut nested);
            on_search_changed(app);
        }
        Request::CommitSearch => {
            let ctx = ctx(app);
            let outs = route_scroll_current(app, ctx);
            split_outs(outs, cmds, &mut nested);
        }
        Request::ClearSearch => {
            let ctx = ctx(app);
            let outs = route_clear_search(app, ctx);
            split_outs(outs, cmds, &mut nested);
            on_search_changed(app);
        }
        Request::Focus(focus) => app.loaded.set_focus(focus),
        // The file tree is the only navigator, so it is always the default.
        Request::FocusDefault => app.loaded.set_focus(Focus::Tree),
        Request::CycleFocus { forward } => {
            let ctx = ctx(app);
            let outs = route_cycle_focus(app, forward, ctx);
            split_outs(outs, cmds, &mut nested);
        }
        Request::ResizeNavigator(resize) => {
            app.chrome.tree_percent = match resize {
                NavigatorResize::Wider => {
                    (app.chrome.tree_percent + TREE_STEP).min(TREE_MAX_PERCENT)
                }
                NavigatorResize::Narrower => app
                    .chrome
                    .tree_percent
                    .saturating_sub(TREE_STEP)
                    .max(TREE_MIN_PERCENT),
                NavigatorResize::Reset => TREE_DEFAULT_PERCENT,
            };
        }
        Request::CancelPrefix => {
            if app.keymap.pending.is_some() {
                clear_prefix(app);
            }
        }
        Request::RunCommand(id) => {
            if let Some(spec) = crate::command::commands(app)
                .into_iter()
                .find(|spec| spec.id == id)
            {
                apply_action(app, spec.run, cmds);
            }
        }
    }
    nested
}

/// Route a page's outs: effects append to `cmds` in order, requests nest.
fn split_outs(outs: Vec<OutMsg>, cmds: &mut Vec<Cmd>, nested: &mut Vec<Request>) {
    for out in outs {
        match out {
            OutMsg::Effect(cmd) => cmds.push(cmd),
            OutMsg::Request(request) => nested.push(request),
        }
    }
}

/// Set the selection, mirroring the old `select_path`: clear a pending prefix,
/// select the path, reveal it in the tree, and reset the body scroll.
fn select(app: &mut App, selection: Option<RepoPath>) {
    clear_prefix(app);
    match selection {
        Some(path) => {
            app.loaded.set_selected(Some(path.clone()));
            let height = tree_viewport_height(app);
            app.tree.reveal(&path, height);
        }
        None => app.loaded.set_selected(None),
    }
    app.loaded.reset_body_scroll();
}

/// The file set changed. In 2.1 this is a seam only; task 2.2 gives the tree
/// its selection fallback here.
fn on_files_changed(_app: &mut App) {}

/// The selection changed. In 2.1 this is a seam only; later tasks reveal the
/// new selection in the tree here. It fires for every `Select`, even when the
/// path is unchanged.
fn on_selection_changed(_app: &mut App) {}

/// The committed query changed. In 2.1 this is a seam only.
fn on_query_changed(_app: &mut App) {}

/// The search changed. In 2.1 this is a seam only.
fn on_search_changed(_app: &mut App) {}

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

fn install_show_selection(app: &mut App) {
    let height = tree_viewport_height(app);
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
            app.tree.reveal(&path, height);
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
        cursor_line: 1,
    };
    show.ensure_visible_rows();
}

fn install_diff_selection(app: &mut App) {
    let height = tree_viewport_height(app);
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
            app.tree.reveal(&path, height);
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

fn handle_key(app: &mut App, key: Key, cmds: &mut Vec<Cmd>) {
    // 1. Ctrl-C always quits.
    if key == Key::CtrlC {
        app.chrome.quit = true;
        return;
    }
    // 2. Ctrl-K/Ctrl-J were k/j before the band; keep that alias outside it.
    let key = if app.overlay.as_ref().is_some_and(Overlay::keeps_ctrl_jk) {
        key
    } else {
        match key {
            Key::CtrlK => Key::Char('k'),
            Key::CtrlJ => Key::Char('j'),
            other => other,
        }
    };
    // 3. An open overlay owns the key.
    if let Some((captures_text, modal)) =
        app.overlay.as_ref().map(|o| (o.captures_text(), o.modal()))
    {
        // 3a. `q` quits unless the overlay captures text (or a prompt does).
        if key == Key::Char('q') && !(captures_text || app.loaded.prompt_active()) {
            app.chrome.quit = true;
            return;
        }
        // 3b. Route the key to the overlay and apply its outs.
        let (handled, outs) = overlay_key(app, key);
        for out in outs {
            apply_overlay_out(app, out, cmds);
        }
        // 3c. A handled key, or any modal overlay, stops here.
        if handled || modal {
            return;
        }
    }
    // Prompts are not overlays until task 4.4; route to them as before.
    if app.loaded.prompt_active() {
        let ctx = ctx(app);
        let outs = route_prompt(app, key, ctx);
        fold(app, outs, cmds);
        return;
    }
    // 4. A pending prefix resolves or cancels.
    if let Some(prefix) = app.keymap.pending {
        clear_prefix(app);
        if let Key::Char(character) = key
            && let Some(action) = prefixed_action(app, prefix, character)
        {
            apply_action(app, action, cmds);
        }
        return;
    }
    // 5. `q` quits.
    if key == Key::Char('q') {
        app.chrome.quit = true;
        return;
    }
    // 6. A prefix key arms the band.
    if let Some(prefix) = crate::command::pending_prefix(app, key) {
        app.keymap.pending = Some(prefix);
        app.overlay = Some(Overlay::which_key());
        return;
    }
    // 7. A bound key runs its command.
    if let Some(action) = command_for(key, app) {
        apply_action(app, action, cmds);
        return;
    }
    // 8. Enter folds in the show body; otherwise forward to the focused pane.
    if key == Key::Enter && show_body_focused(app) {
        apply_action(app, Action::Show(ShowAction::ToggleFold), cmds);
        return;
    }
    if focus_key(app, key, cmds) {
        return;
    }
    // 9. Esc clears the diagnostic and the search.
    if key == Key::Esc {
        app.diagnostic = None;
        let ctx = ctx(app);
        let outs = route_clear_search(app, ctx);
        fold(app, outs, cmds);
    }
}

fn show_body_focused(app: &App) -> bool {
    matches!(app.loaded, Loaded::Show(_)) && app.loaded.focus() == Focus::Content
}

/// The action a `Prefixed(prefix, key)` binding in scope runs, if any.
fn prefixed_action(app: &App, prefix: char, key: char) -> Option<Action> {
    crate::command::prefix_groups_in_scope(&app.loaded, Some(app.loaded.focus()))
        .iter()
        .find(|group| group.key == prefix)
        .and_then(|group| group.bindings.iter().find(|binding| binding.key == key))
        .map(|binding| binding.action)
}

fn clear_prefix(app: &mut App) {
    app.keymap.pending = None;
    if matches!(app.overlay, Some(Overlay::WhichKey { .. })) {
        app.overlay = None;
    }
}

fn command_for(key: Key, app: &App) -> Option<Action> {
    crate::command::action_for_key(app, key)
}

fn apply_action(app: &mut App, action: Action, cmds: &mut Vec<Cmd>) {
    let ctx = ctx(app);
    match action {
        Action::Global(action) => apply_global(app, action, cmds),
        Action::Show(ShowAction::EditRevision) => {
            let outs = route_show(app, show_page::Msg::EditRevision, ctx);
            fold(app, outs, cmds);
        }
        Action::Show(action) => {
            let outs = route_show(app, show_page::Msg::Fold(action), ctx);
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

/// Open `overlay`, defensively clearing any pending prefix so the invariant
/// `keymap.pending.is_some() == (overlay is WhichKey)` holds.
fn open_overlay(app: &mut App, overlay: Overlay) {
    app.keymap.pending = None;
    app.overlay = Some(overlay);
}

fn apply_global(app: &mut App, action: GlobalAction, cmds: &mut Vec<Cmd>) {
    use GlobalAction::*;
    let ctx = ctx(app);
    match action {
        Quit => app.chrome.quit = true,
        Help => open_overlay(app, Overlay::help()),
        Scope => {
            open_overlay(app, Overlay::scope());
            cmds.push(Cmd::LoadAreas);
        }
        Mode => open_overlay(app, Overlay::mode(app.loaded.mode())),
        Palette => {
            let entries = app.loaded.palette_entries();
            let groups = app.loaded.prefix_groups();
            let mode = app.loaded.mode();
            let visible = app.loaded.visible();
            let pending_prefix = app.keymap.pending;
            let palette = Overlay::palette(&overlay::Ctx {
                visible,
                entries: &entries,
                groups: &groups,
                mode,
                pending_prefix,
            });
            open_overlay(app, palette);
        }
        Finder => {
            let entries = app.loaded.palette_entries();
            let groups = app.loaded.prefix_groups();
            let mode = app.loaded.mode();
            let visible = app.loaded.visible();
            let pending_prefix = app.keymap.pending;
            let finder = Overlay::finder(&overlay::Ctx {
                visible,
                entries: &entries,
                groups: &groups,
                mode,
                pending_prefix,
            });
            open_overlay(app, finder);
        }
        Search => open_overlay(app, Overlay::search()),
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

/// Forward `key` to the focused pane, as the old `focus_key` did. Returns
/// whether the pane consumed the key, so an unhandled `Esc` can clear the
/// diagnostic and the search (step 9).
fn focus_key(app: &mut App, key: Key, cmds: &mut Vec<Cmd>) -> bool {
    if app.loaded.focus() == Focus::Tree {
        if let Some(msg) = tree_key(key) {
            tree_apply(app, msg);
            return true;
        }
        return false;
    }
    let handled = pane_key_handled(app, key);
    let ctx = ctx(app);
    let outs = route_page_key(app, key, ctx);
    fold(app, outs, cmds);
    handled
}

/// The keys a focused body pane consumes today (`page::{show,diff}::body_key`).
fn pane_key_handled(app: &App, key: Key) -> bool {
    match &app.loaded {
        Loaded::Show(_) => matches!(
            key,
            Key::Down
                | Key::Char('j')
                | Key::Up
                | Key::Char('k')
                | Key::Right
                | Key::Char('l')
                | Key::Left
                | Key::Char('h')
        ),
        Loaded::Diff(diff) => match diff.focus() {
            Focus::Content | Focus::Commits => {
                matches!(key, Key::Down | Key::Char('j') | Key::Up | Key::Char('k'))
            }
            Focus::Tree => false,
        },
    }
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
    let height = tree_viewport_height(app);
    let selected = app.tree.update(msg, app.loaded.visible(), height);
    if let Some(tree::OutMsg::Selected(path)) = selected {
        select_path(app, path);
    }
}

fn select_path(app: &mut App, path: RepoPath) {
    select(app, Some(path));
}

fn overlay_update(app: &mut App, msg: overlay::Msg) -> Vec<overlay::OutMsg> {
    let Some(mut current) = app.overlay.take() else {
        return Vec::new();
    };
    let outs = current.update(msg);
    app.overlay = Some(current);
    outs
}

/// Route a key to the open overlay. Returns `(handled, outs)`; `handled` is
/// false only when a non-modal overlay declines the key.
fn overlay_key(app: &mut App, key: Key) -> (bool, Vec<overlay::OutMsg>) {
    let Some(mut current) = app.overlay.take() else {
        return (false, Vec::new());
    };
    let entries = app.loaded.palette_entries();
    let groups = app.loaded.prefix_groups();
    let mode = app.loaded.mode();
    let visible = app.loaded.visible();
    let pending_prefix = app.keymap.pending;
    let ctx = overlay::Ctx {
        visible,
        entries: &entries,
        groups: &groups,
        mode,
        pending_prefix,
    };
    let result = current.key(key, &ctx);
    app.overlay = Some(current);
    match result {
        Some(outs) => (true, outs),
        None => (false, Vec::new()),
    }
}

fn apply_overlay_out(app: &mut App, out: overlay::OutMsg, cmds: &mut Vec<Cmd>) {
    let ctx = ctx(app);
    match out {
        overlay::OutMsg::Close => {
            app.overlay = None;
            app.keymap.pending = None;
        }
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
                    let index = app.tree.scroll + usize::from(event.row.saturating_sub(inner.y));
                    if index < app.tree.rows.len() {
                        tree_apply(app, tree::Msg::Click(index));
                    }
                }
                MouseKind::ScrollUp => tree_apply(app, tree::Msg::Scroll(-MOUSE_SCROLL_STEP)),
                MouseKind::ScrollDown => tree_apply(app, tree::Msg::Scroll(MOUSE_SCROLL_STEP)),
            }
        }
        PaneSlot::Show => {
            app.loaded.set_focus(Focus::Content);
            match event.kind {
                MouseKind::Click => {
                    if inner.contains(point)
                        && let Loaded::Show(show) = &mut app.loaded
                    {
                        let row = show.body.scroll as usize
                            + usize::from(event.row.saturating_sub(inner.y));
                        if row < show.line_count() {
                            let marker = event.column - inner.x < show_page::MARKER_CLICK_WIDTH;
                            let outs = show_page::update(
                                show_page::Msg::BodyClick { row, marker },
                                show,
                                &ctx,
                            );
                            fold(app, outs, cmds);
                        }
                    }
                }
                MouseKind::ScrollUp | MouseKind::ScrollDown => {
                    let delta = if event.kind == MouseKind::ScrollUp {
                        -MOUSE_SCROLL_STEP
                    } else {
                        MOUSE_SCROLL_STEP
                    };
                    if let Loaded::Show(show) = &mut app.loaded {
                        let outs = show_page::update(show_page::Msg::Scroll(delta), show, &ctx);
                        fold(app, outs, cmds);
                    }
                }
            }
        }
        PaneSlot::Old | PaneSlot::New => {
            app.loaded.set_focus(Focus::Content);
            let delta = match event.kind {
                MouseKind::ScrollUp => -MOUSE_SCROLL_STEP,
                MouseKind::ScrollDown => MOUSE_SCROLL_STEP,
                MouseKind::Click => 0,
            };
            if delta != 0
                && let Loaded::Diff(diff) = &mut app.loaded
            {
                let outs = diff_page::update(diff_page::Msg::Scroll(delta), diff, &ctx);
                fold(app, outs, cmds);
            }
        }
    }
}

fn tree_inner(app: &App) -> Option<Rect> {
    let (_, content, _) = frame_areas(app.size.width, app.size.height);
    let is_diff = matches!(app.loaded, Loaded::Diff(_));
    let has_commits =
        matches!(&app.loaded, Loaded::Diff(diff) if matches!(diff.view, DiffViewState::Commits(_)));
    let layout = body_layout(
        content,
        app.chrome.tree_percent,
        is_diff,
        has_commits,
        Focus::Tree,
    );
    let slot = layout
        .slots
        .iter()
        .find(|slot| slot.pane == PaneSlot::Tree)?;
    Some(pane_block("", false, &app.chrome.theme, slot.edge).inner(slot.outer))
}

fn tree_viewport_height(app: &App) -> usize {
    tree_inner(app).map_or(0, |rect| rect.height as usize)
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

fn step_in_range(app: &App, index: usize) -> bool {
    match &app.loaded {
        Loaded::Diff(diff) => match &diff.view {
            DiffViewState::Commits(commits) => index < commits.picker.steps_slice().len(),
            DiffViewState::Range(_) => false,
        },
        Loaded::Show(_) => false,
    }
}

fn body_is_visible(app: &App) -> bool {
    if app.size.width < SINGLE_PANE_MIN_WIDTH || app.size.height < MIN_HEIGHT {
        return false;
    }
    app.size.width >= SIDE_BY_SIDE_MIN_WIDTH || app.loaded.focus() != Focus::Tree
}

pub(crate) fn layout_focus(app: &App) -> Focus {
    app.loaded.focus()
}

pub(crate) fn settle(mut app: App) -> App {
    let visible_body = body_is_visible(&app);
    let width = app.size.width;
    let height = app.size.height;
    let tree_percent = app.chrome.tree_percent;
    app.loaded
        .settle(&app.chrome.theme, visible_body, width, height, tree_percent);
    let tree_height = tree_viewport_height(&app);
    app.tree.fit_viewport(tree_height);
    app
}

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
