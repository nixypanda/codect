//! The app shell and composer: the one place that owns shared state, routes
//! input to the component that owns it, and folds the component `OutMsg`s back
//! into shared state and effects.
//!
//! Nothing here performs I/O. `update` turns a message and the current model
//! into a replacement model plus a list of effects to run. Engine calls only
//! ever leave this module as a [`Cmd`]. The pure `view` lives in [`crate::view`].

use std::sync::Arc;

use base::{AreaSet, FileDiff, ProjectedFile, ProjectionMode, RepoPath, Selection};
use engine::{CommitStep, EngineError};
use ratatui::layout::{Position, Rect};

use crate::action::{Action, CommitsAction, GlobalAction, RangeAction, ShowAction};
use crate::components::commit_picker::{self, CommitPicker};
use crate::components::overlay::{self, Overlay};
use crate::components::text_input::{Edit, TextInput};
use crate::components::tree::{self, RowKind, Tree};
use crate::content::{
    self, Commits, CommitsFocus, Diff, DiffBody, DiffHighlight, DiffPrompt, DiffSide,
    DiffViewState, Loaded, Paging, Range, RangeFocus, Scope, SearchMatch, SearchSide, Show,
    ShowBody, ShowFocus,
};
use crate::icons::Icons;
use crate::input::{Key, Mouse, MouseKind};
use crate::layout::{
    Edge, PaneSlot, VisualRow, body_layout, commit_offset, frame_areas, gutter_width, layout_diff,
    split_with_dividers, window_offset,
};
use crate::theme::Theme;
use crate::view::block::pane_block;

/// Below this width, or below [`MIN_HEIGHT`] rows, the terminal is too small.
pub(crate) const SINGLE_PANE_MIN_WIDTH: u16 = 40;
pub(crate) const MIN_HEIGHT: u16 = 8;
pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = content::SIDE_BY_SIDE_MIN_WIDTH;

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

/// Which body pane has focus, normalized for layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Focus {
    Commits,
    Tree,
    Content,
}

/// The projection to open with. A session's kind is fixed by this request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadRequest {
    Show {
        revision: String,
        mode: ProjectionMode,
        selection: Selection,
    },
    Diff {
        base: String,
        target: String,
        mode: ProjectionMode,
        selection: Selection,
        view: DiffView,
    },
}

/// Which diff view to open or reload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffView {
    Range,
    Commits,
}

/// A concrete `show` effect request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShowRequest {
    pub revision: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
}

/// A concrete `diff` effect request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRequest {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
    pub view: DiffView,
}

/// A commits-view effect payload: the steps and the first step's diff.
pub type HistoryPayload = (Arc<[CommitStep]>, Arc<[FileDiff]>);

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

/// An effect the runtime must interpret. I/O is data, never a side effect of
/// `update`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    Show(ShowRequest),
    Diff(DiffRequest),
    Step {
        request: DiffRequest,
        index: usize,
        step: CommitStep,
    },
    LoadAreas,
}

/// The inputs a cached diff layout depends on.
#[derive(Clone, PartialEq, Eq)]
struct DerivedKey {
    generation: u64,
    width: u16,
    height: u16,
    selected: Option<RepoPath>,
    tree_percent: u16,
}

/// Expensive rendering state derived from the model, cached so a frame or a
/// scroll key does not recompute it. Held behind an `Arc`, so cloning a `Model`
/// never clones the layout.
#[derive(Clone, Default)]
struct Derived {
    key: Option<DerivedKey>,
    diff_rows: Vec<VisualRow>,
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
    /// Bumped whenever the loaded content is replaced, invalidating [`Derived`].
    generation: u64,
    derived: Arc<Derived>,
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
            } => Loaded::Diff(Diff {
                base,
                target,
                mode,
                scope: Scope {
                    selection,
                    label: scope_label,
                },
                visible: Arc::from(Vec::new()),
                selected: None,
                search: None,
                paging: Paging::Loading,
                prompt: None,
                view: match view {
                    DiffView::Range => DiffViewState::Range(Range::empty()),
                    DiffView::Commits => DiffViewState::Commits(Commits::empty()),
                },
            }),
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
            generation: 0,
            derived: Arc::new(Derived::default()),
        }
    }

    /// Whether a projection is in flight, so the view can spin.
    pub fn is_busy(&self) -> bool {
        self.loaded.is_loading()
    }

    pub(crate) fn diff_rows(&self) -> &[VisualRow] {
        self.derived.diff_rows.as_slice()
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

    match msg {
        Msg::Key(key) => handle_key(&mut next, key, &mut cmds),
        Msg::Mouse(event) => mouse(&mut next, event, &mut cmds),
        Msg::Resize { width, height } => {
            next.size = Size { width, height };
            clamp_view(&mut next);
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
        Msg::ShowLoaded { request, result } => show_loaded(&mut next, request, result),
        Msg::DiffLoaded { request, result } => diff_loaded(&mut next, request, result),
        Msg::HistoryLoaded { request, result } => {
            history_loaded(&mut next, request, result, &mut cmds)
        }
        Msg::StepLoaded { index, result } => step_loaded(&mut next, index, result),
        Msg::AreasLoaded(result) => {
            let outs = overlay_update(&mut next, overlay::Msg::AreasLoaded(result));
            for out in outs {
                apply_overlay_out(&mut next, out, &mut cmds);
            }
        }
    }

    (next, cmds)
}

fn show_loaded(
    app: &mut App,
    request: ShowRequest,
    result: Result<Arc<[ProjectedFile]>, Box<EngineError>>,
) {
    let Loaded::Show(show) = &mut app.loaded else {
        return;
    };
    match result {
        Ok(files) => {
            show.paging = Paging::Ready;
            show.revision = request.revision;
            show.mode = request.mode;
            show.scope = Scope::new(request.selection);
            show.files = files;
            show.visible = Arc::from(content::show_visible(&show.files));
            show.highlight.clear();
            show.prompt = None;
            app.generation = app.generation.wrapping_add(1);
            install_show_selection(app);
            app.diagnostic = None;
        }
        Err(error) => {
            show.paging = Paging::Ready;
            app.diagnostic = Some(Diagnostic::new(error.to_string()));
        }
    }
}

fn diff_loaded(
    app: &mut App,
    request: DiffRequest,
    result: Result<Arc<[FileDiff]>, Box<EngineError>>,
) {
    let Loaded::Diff(diff) = &mut app.loaded else {
        return;
    };
    match result {
        Ok(diffs) => {
            diff.paging = Paging::Ready;
            diff.base = request.base;
            diff.target = request.target;
            diff.mode = request.mode;
            diff.scope = Scope::new(request.selection);
            diff.prompt = None;
            if !matches!(diff.view, DiffViewState::Range(_)) {
                diff.view = DiffViewState::Range(Range::empty());
            }
            if let DiffViewState::Range(range) = &mut diff.view {
                range.diffs = diffs;
                range.highlight.clear();
            }
            diff.visible = Arc::from(content::diff_visible(diff.diffs()));
            app.generation = app.generation.wrapping_add(1);
            install_diff_selection(app);
            app.diagnostic = None;
        }
        Err(error) => {
            diff.paging = Paging::Ready;
            app.diagnostic = Some(Diagnostic::new(error.to_string()));
        }
    }
}

fn history_loaded(
    app: &mut App,
    request: DiffRequest,
    result: Result<HistoryPayload, Box<EngineError>>,
    cmds: &mut Vec<Cmd>,
) {
    let Loaded::Diff(diff) = &mut app.loaded else {
        return;
    };
    match result {
        Ok((steps, diffs)) => {
            diff.paging = Paging::Ready;
            diff.base = request.base;
            diff.target = request.target;
            diff.mode = request.mode;
            diff.scope = Scope::new(request.selection);
            diff.prompt = None;
            let preferred = match &diff.view {
                DiffViewState::Commits(commits) => commits
                    .picker
                    .steps_slice()
                    .get(commits.picker.cursor())
                    .map(|step| step.commit_id.clone()),
                DiffViewState::Range(_) => None,
            };
            diff.view = DiffViewState::Commits(Commits {
                focus: CommitsFocus::Commits,
                picker: CommitPicker::steps(steps),
                body: DiffBody { scroll: 0 },
                diffs,
                highlight: Default::default(),
            });
            diff.visible = Arc::from(content::diff_visible(diff.diffs()));
            app.generation = app.generation.wrapping_add(1);
            install_diff_selection(app);
            app.diagnostic = None;

            // Restore the same commit as before, if it is still in range.
            if let Loaded::Diff(diff) = &mut app.loaded
                && let DiffViewState::Commits(commits) = &mut diff.view
                && let Some(index) = preferred.and_then(|id| {
                    commits
                        .picker
                        .steps_slice()
                        .iter()
                        .position(|step| step.commit_id == id)
                })
                && index > 0
                && let Some(step) = commits.picker.steps_slice().get(index).cloned()
            {
                diff.paging = Paging::Loading;
                cmds.push(Cmd::Step {
                    request: diff_request(diff),
                    index,
                    step,
                });
            }
        }
        Err(error) => {
            diff.paging = Paging::Ready;
            app.diagnostic = Some(Diagnostic::new(error.to_string()));
        }
    }
}

fn step_loaded(app: &mut App, index: usize, result: Result<Arc<[FileDiff]>, Box<EngineError>>) {
    let inner = commits_inner(app);
    let Loaded::Diff(diff) = &mut app.loaded else {
        return;
    };
    let DiffViewState::Commits(commits) = &mut diff.view else {
        return;
    };
    match result {
        Ok(diffs) if index < commits.picker.steps_slice().len() => {
            diff.paging = Paging::Ready;
            commits.diffs = diffs;
            commits.highlight.clear();
            commits.picker.update(commit_picker::Msg::Loaded(index));
            if let Some(inner) = inner {
                commits.picker.set_scroll(commit_offset(
                    commits.picker.cursor(),
                    commits.picker.scroll(),
                    commits.picker.steps_slice().len(),
                    inner.height as usize,
                ));
            }
            diff.visible = Arc::from(content::diff_visible(commits.diffs.as_ref()));
            app.generation = app.generation.wrapping_add(1);
            install_diff_selection(app);
            app.diagnostic = None;
        }
        Ok(_) => {
            commits.picker.update(commit_picker::Msg::Failed);
        }
        Err(error) => {
            diff.paging = Paging::Ready;
            commits.picker.update(commit_picker::Msg::Failed);
            app.diagnostic = Some(Diagnostic::new(error.to_string()));
        }
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

fn diff_request(diff: &Diff) -> DiffRequest {
    DiffRequest {
        base: diff.base.clone(),
        target: diff.target.clone(),
        mode: diff.mode,
        selection: diff.scope.selection.clone(),
        view: match diff.view {
            DiffViewState::Range(_) => DiffView::Range,
            DiffViewState::Commits(_) => DiffView::Commits,
        },
    }
}

fn show_request(show: &Show) -> ShowRequest {
    ShowRequest {
        revision: show.revision.clone(),
        mode: show.mode,
        selection: show.scope.selection.clone(),
    }
}

// ---------------------------------------------------------------------------
// Key routing
// ---------------------------------------------------------------------------

fn handle_key(app: &mut App, key: Key, cmds: &mut Vec<Cmd>) {
    if key == Key::CtrlC {
        app.chrome.quit = true;
        return;
    }
    let capturing = app.overlay.as_ref().is_some_and(Overlay::captures_text) || prompt_active(app);
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
    if prompt_active(app) {
        prompt_key(app, key, cmds);
        return;
    }
    if let Some(action) = command_for(key, app) {
        apply_action(app, action, cmds);
        return;
    }
    match key {
        Key::Esc => {
            app.diagnostic = None;
            clear_search(app);
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
    match action {
        Action::Global(action) => apply_global(app, action, cmds),
        Action::Show(ShowAction::EditRevision) => {
            if let Loaded::Show(show) = &mut app.loaded {
                show.prompt = Some(TextInput::new(show.revision.clone()));
            }
        }
        Action::Range(action) => {
            let Loaded::Diff(diff) = &mut app.loaded else {
                return;
            };
            if !matches!(diff.view, DiffViewState::Range(_)) {
                return;
            }
            match action {
                RangeAction::EditBase => {
                    diff.prompt = Some(DiffPrompt {
                        side: DiffSide::Base,
                        input: TextInput::new(diff.base.clone()),
                    });
                }
                RangeAction::EditTarget => {
                    diff.prompt = Some(DiffPrompt {
                        side: DiffSide::Target,
                        input: TextInput::new(diff.target.clone()),
                    });
                }
                RangeAction::SwitchToCommits => {
                    diff.paging = Paging::Loading;
                    let mut request = diff_request(diff);
                    request.view = DiffView::Commits;
                    cmds.push(Cmd::Diff(request));
                }
            }
        }
        Action::Commits(CommitsAction::SwitchToRange) => {
            let Loaded::Diff(diff) = &mut app.loaded else {
                return;
            };
            if !matches!(diff.view, DiffViewState::Commits(_)) {
                return;
            }
            diff.paging = Paging::Loading;
            let mut request = diff_request(diff);
            request.view = DiffView::Range;
            cmds.push(Cmd::Diff(request));
        }
    }
}

fn apply_global(app: &mut App, action: GlobalAction, cmds: &mut Vec<Cmd>) {
    use GlobalAction::*;
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
        NextPane => cycle_focus(app, true),
        PreviousPane => cycle_focus(app, false),
        Top | Bottom | PageUp | PageDown => focus_command(app, action, cmds),
        NextMatch => step_search(app, true),
        PreviousMatch => step_search(app, false),
    }
}

fn focus_command(app: &mut App, action: GlobalAction, cmds: &mut Vec<Cmd>) {
    let _ = cmds;
    let step = page_step(app);
    let max = max_scroll(app);
    let nav = command_intent(app, action, step, max);
    apply_intent(app, nav);
}

fn command_intent(app: &App, action: GlobalAction, step: u16, max: u16) -> Option<Intent> {
    match &app.loaded {
        Loaded::Show(show) => match show.focus {
            ShowFocus::Tree => Some(Intent::Tree(page_tree_msg(action, step))),
            ShowFocus::Body => Some(Intent::ShowBody(ShowBody {
                scroll: page_scroll(action, show.body.scroll, step, max),
                ..show.body
            })),
        },
        Loaded::Diff(diff) => match diff_focus(diff) {
            Focus::Commits => Some(Intent::Commit(commit_move_msg(action, step))),
            Focus::Tree => Some(Intent::Tree(page_tree_msg(action, step))),
            Focus::Content => Some(Intent::DiffScroll(page_scroll(
                action,
                diff.body_scroll(),
                step,
                max,
            ))),
        },
    }
}

fn page_scroll(action: GlobalAction, current: u16, step: u16, max: u16) -> u16 {
    match action {
        GlobalAction::Top => 0,
        GlobalAction::Bottom => max,
        GlobalAction::PageUp => current.saturating_sub(step),
        _ => current.saturating_add(step).min(max),
    }
}

fn page_tree_msg(action: GlobalAction, step: u16) -> tree::Msg {
    match action {
        GlobalAction::Top => tree::Msg::ToTop,
        GlobalAction::Bottom => tree::Msg::ToBottom,
        GlobalAction::PageUp => tree::Msg::Page(-i32::from(step)),
        _ => tree::Msg::Page(i32::from(step)),
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

fn focus_key(app: &mut App, key: Key, _cmds: &mut Vec<Cmd>) {
    let max = max_scroll(app);
    let max_h = max_hscroll(app);
    let nav = nav_intent(app, key, max, max_h);
    apply_intent(app, nav);
}

fn nav_intent(app: &App, key: Key, max: u16, max_h: u16) -> Option<Intent> {
    match &app.loaded {
        Loaded::Show(show) => match show.focus {
            ShowFocus::Tree => tree_key(key).map(Intent::Tree),
            ShowFocus::Body => body_key(key, show.body, max, max_h).map(Intent::ShowBody),
        },
        Loaded::Diff(diff) => match diff_focus(diff) {
            Focus::Commits => commit_key(key).map(Intent::Commit),
            Focus::Tree => tree_key(key).map(Intent::Tree),
            Focus::Content => {
                let scroll = body_scroll_key(key, diff.body_scroll(), max);
                (scroll != diff.body_scroll()).then_some(Intent::DiffScroll(scroll))
            }
        },
    }
}

/// A navigation intent resolved from the current focus, applied after the
/// borrow of the loaded projection ends.
enum Intent {
    Tree(tree::Msg),
    Commit(commit_picker::Msg),
    ShowBody(ShowBody),
    DiffScroll(u16),
}

fn apply_intent(app: &mut App, intent: Option<Intent>) {
    match intent {
        Some(Intent::Tree(msg)) => tree_apply(app, msg),
        Some(Intent::Commit(msg)) => commit_apply(app, msg),
        Some(Intent::ShowBody(body)) => {
            if let Loaded::Show(show) = &mut app.loaded {
                show.body = body;
            }
        }
        Some(Intent::DiffScroll(scroll)) => {
            if let Loaded::Diff(diff) = &mut app.loaded {
                diff.set_body_scroll(scroll);
            }
        }
        None => {}
    }
}

fn body_scroll_key(key: Key, scroll: u16, max: u16) -> u16 {
    match key {
        Key::Down | Key::Char('j') => scroll.saturating_add(1).min(max),
        Key::Up | Key::Char('k') => scroll.saturating_sub(1),
        _ => scroll,
    }
}

fn body_key(key: Key, body: ShowBody, max: u16, max_h: u16) -> Option<ShowBody> {
    match key {
        Key::Down | Key::Char('j') => Some(ShowBody {
            scroll: body.scroll.saturating_add(1).min(max),
            ..body
        }),
        Key::Up | Key::Char('k') => Some(ShowBody {
            scroll: body.scroll.saturating_sub(1),
            ..body
        }),
        Key::Right | Key::Char('l') => Some(ShowBody {
            hscroll: body.hscroll.saturating_add(1).min(max_h),
            ..body
        }),
        Key::Left | Key::Char('h') => Some(ShowBody {
            hscroll: body.hscroll.saturating_sub(1),
            ..body
        }),
        _ => None,
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

fn commit_key(key: Key) -> Option<commit_picker::Msg> {
    match key {
        Key::Down | Key::Char('j') => Some(commit_picker::Msg::Move(1)),
        Key::Up | Key::Char('k') => Some(commit_picker::Msg::Move(-1)),
        _ => None,
    }
}

fn tree_apply(app: &mut App, msg: tree::Msg) {
    let selected = app.tree.update(msg, app.loaded.visible());
    if let Some(tree::OutMsg::Selected(path)) = selected {
        select_path(app, path);
    }
}

fn commit_apply(app: &mut App, msg: commit_picker::Msg) {
    if let Loaded::Diff(diff) = &mut app.loaded
        && let DiffViewState::Commits(commits) = &mut diff.view
    {
        commits.picker.update(msg);
    }
}

fn select_path(app: &mut App, path: RepoPath) {
    app.loaded.set_selected(Some(path.clone()));
    app.tree.reveal(&path);
    reset_body_scroll(app);
}

fn reset_body_scroll(app: &mut App) {
    match &mut app.loaded {
        Loaded::Show(show) => {
            show.body = ShowBody {
                scroll: 0,
                hscroll: 0,
            }
        }
        Loaded::Diff(diff) => diff.set_body_scroll(0),
    }
}

fn clear_search(app: &mut App) {
    match &mut app.loaded {
        Loaded::Show(show) => show.search = None,
        Loaded::Diff(diff) => diff.search = None,
    }
}

fn set_search(app: &mut App, needle: &str) {
    match &mut app.loaded {
        Loaded::Show(show) => show.set_search(needle),
        Loaded::Diff(diff) => diff.set_search(needle),
    }
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
    match out {
        overlay::OutMsg::Close => app.overlay = None,
        overlay::OutMsg::Selection(selection) => reload_scope(app, selection, cmds),
        overlay::OutMsg::Mode(mode) => reload_mode(app, mode, cmds),
        overlay::OutMsg::Action(action) => apply_action(app, action, cmds),
        overlay::OutMsg::File(path) => select_path(app, path),
        overlay::OutMsg::Search(needle) => set_search(app, &needle),
        overlay::OutMsg::SearchCommit => focus_current_match(app),
    }
}

fn reload_scope(app: &mut App, selection: Selection, cmds: &mut Vec<Cmd>) {
    match &mut app.loaded {
        Loaded::Show(show) => {
            show.paging = Paging::Loading;
            let mut request = show_request(show);
            request.selection = selection;
            cmds.push(Cmd::Show(request));
        }
        Loaded::Diff(diff) => {
            diff.paging = Paging::Loading;
            let mut request = diff_request(diff);
            request.selection = selection;
            cmds.push(Cmd::Diff(request));
        }
    }
}

fn reload_mode(app: &mut App, mode: ProjectionMode, cmds: &mut Vec<Cmd>) {
    match &mut app.loaded {
        Loaded::Show(show) => {
            show.paging = Paging::Loading;
            let mut request = show_request(show);
            request.mode = mode;
            cmds.push(Cmd::Show(request));
        }
        Loaded::Diff(diff) => {
            diff.paging = Paging::Loading;
            let mut request = diff_request(diff);
            request.mode = mode;
            cmds.push(Cmd::Diff(request));
        }
    }
}

fn prompt_active(app: &App) -> bool {
    match &app.loaded {
        Loaded::Show(show) => show.prompt.is_some(),
        Loaded::Diff(diff) => diff.prompt.is_some(),
    }
}

fn prompt_key(app: &mut App, key: Key, cmds: &mut Vec<Cmd>) {
    match &mut app.loaded {
        Loaded::Show(show) => {
            let Some(mut input) = show.prompt.take() else {
                return;
            };
            let mut reopen = true;
            match key {
                Key::Esc => reopen = false,
                Key::Enter => {
                    reopen = false;
                    let value = input.value();
                    if !value.is_empty() {
                        let request = ShowRequest {
                            revision: value,
                            mode: show.mode,
                            selection: show.scope.selection.clone(),
                        };
                        show.paging = Paging::Loading;
                        cmds.push(Cmd::Show(request));
                    }
                }
                key => {
                    if let Some(edit) = text_edit(key) {
                        input.edit(edit);
                    }
                }
            }
            if reopen {
                show.prompt = Some(input);
            }
        }
        Loaded::Diff(diff) => {
            let Some(mut prompt) = diff.prompt.take() else {
                return;
            };
            let mut reopen = true;
            match key {
                Key::Esc => reopen = false,
                Key::Enter => {
                    reopen = false;
                    let value = input_value(&prompt.input);
                    if !value.is_empty() {
                        let mut request = diff_request(diff);
                        match prompt.side {
                            DiffSide::Base => request.base = value,
                            DiffSide::Target => request.target = value,
                        }
                        diff.paging = Paging::Loading;
                        cmds.push(Cmd::Diff(request));
                    }
                }
                key => {
                    if let Some(edit) = text_edit(key) {
                        prompt.input.edit(edit);
                    }
                }
            }
            if reopen {
                diff.prompt = Some(prompt);
            }
        }
    }
}

fn input_value(input: &TextInput) -> String {
    input.value()
}

fn text_edit(key: Key) -> Option<Edit> {
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
// Focus
// ---------------------------------------------------------------------------

fn diff_focus(diff: &Diff) -> Focus {
    match &diff.view {
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

pub(crate) fn layout_focus(app: &App) -> Focus {
    match &app.loaded {
        Loaded::Show(show) => match show.focus {
            ShowFocus::Tree => Focus::Tree,
            ShowFocus::Body => Focus::Content,
        },
        Loaded::Diff(diff) => diff_focus(diff),
    }
}

fn cycle_focus(app: &mut App, forward: bool) {
    match &mut app.loaded {
        Loaded::Show(show) => {
            show.focus = if forward {
                ShowFocus::Body
            } else {
                ShowFocus::Tree
            };
        }
        Loaded::Diff(diff) => match &mut diff.view {
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
        },
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
        layout_focus(app),
    );
    let point = Position {
        x: event.column,
        y: event.row,
    };
    let Some(slot) = layout.slots.iter().find(|slot| slot.outer.contains(point)) else {
        return;
    };
    let inner = pane_block("", false, &app.chrome.theme, slot.edge).inner(slot.outer);

    match slot.pane {
        PaneSlot::Commits => {
            set_focus_commits(app);
            match event.kind {
                MouseKind::Click => {
                    if inner.contains(point)
                        && let Some(index) = commit_click_index(app, inner, event.row)
                    {
                        commit_apply(app, commit_picker::Msg::Select(index));
                    }
                }
                MouseKind::ScrollUp => {
                    commit_apply(app, commit_picker::Msg::Move(-MOUSE_SCROLL_STEP))
                }
                MouseKind::ScrollDown => {
                    commit_apply(app, commit_picker::Msg::Move(MOUSE_SCROLL_STEP))
                }
            }
        }
        PaneSlot::Tree => {
            set_focus_tree(app);
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
            set_focus_content(app);
            match event.kind {
                MouseKind::Click => {}
                MouseKind::ScrollUp => scroll_content(app, -MOUSE_SCROLL_STEP),
                MouseKind::ScrollDown => scroll_content(app, MOUSE_SCROLL_STEP),
            }
        }
    }
    let _ = cmds;
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

fn set_focus_commits(app: &mut App) {
    if let Loaded::Diff(diff) = &mut app.loaded
        && let DiffViewState::Commits(commits) = &mut diff.view
    {
        commits.focus = CommitsFocus::Commits;
    }
}

fn set_focus_tree(app: &mut App) {
    match &mut app.loaded {
        Loaded::Show(show) => show.focus = ShowFocus::Tree,
        Loaded::Diff(diff) => match &mut diff.view {
            DiffViewState::Range(range) => range.focus = RangeFocus::Tree,
            DiffViewState::Commits(commits) => commits.focus = CommitsFocus::Tree,
        },
    }
}

fn set_focus_content(app: &mut App) {
    match &mut app.loaded {
        Loaded::Show(show) => show.focus = ShowFocus::Body,
        Loaded::Diff(diff) => match &mut diff.view {
            DiffViewState::Range(range) => range.focus = RangeFocus::Diff,
            DiffViewState::Commits(commits) => commits.focus = CommitsFocus::Diff,
        },
    }
}

fn scroll_content(app: &mut App, delta: i32) {
    let max = max_scroll(app);
    match &mut app.loaded {
        Loaded::Show(show) => {
            let scroll = if delta < 0 {
                show.body.scroll.saturating_sub(delta.unsigned_abs() as u16)
            } else {
                show.body.scroll.saturating_add(delta as u16).min(max)
            };
            show.body.scroll = scroll;
        }
        Loaded::Diff(diff) => {
            let scroll = diff.body_scroll();
            let next = if delta < 0 {
                scroll.saturating_sub(delta.unsigned_abs() as u16)
            } else {
                scroll.saturating_add(delta as u16).min(max)
            };
            diff.set_body_scroll(next);
        }
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

fn step_search(app: &mut App, forward: bool) {
    let matched = {
        let Some(search) = app.loaded.search() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }
        let len = search.matches.len();
        let cursor = if forward {
            (search.cursor + 1) % len
        } else {
            (search.cursor + len - 1) % len
        };
        (cursor, search.matches[cursor].clone())
    };
    match &mut app.loaded {
        Loaded::Show(show) => {
            if let Some(search) = &mut show.search {
                search.cursor = matched.0;
            }
        }
        Loaded::Diff(diff) => {
            if let Some(search) = &mut diff.search {
                search.cursor = matched.0;
            }
        }
    }
    scroll_to_match(app, &matched.1);
}

fn focus_current_match(app: &mut App) {
    let Some(matched) = app
        .loaded
        .search()
        .and_then(|search| search.matches.get(search.cursor).cloned())
    else {
        return;
    };
    scroll_to_match(app, &matched);
}

fn scroll_to_match(app: &mut App, matched: &SearchMatch) {
    let height = app.size.height.saturating_sub(4) as usize;
    let index = match &app.loaded {
        Loaded::Show(_) => matched.line.saturating_sub(1),
        Loaded::Diff(_) => {
            let side = matched.side;
            app.diff_rows()
                .iter()
                .position(|row| match side {
                    SearchSide::Old => row.old_number == Some(matched.line),
                    SearchSide::New => row.new_number == Some(matched.line),
                    SearchSide::Show => false,
                })
                .unwrap_or(0)
        }
    };
    let scroll = match &app.loaded {
        Loaded::Show(show) => show.body.scroll as usize,
        Loaded::Diff(diff) => diff.body_scroll() as usize,
    };
    let next = if index < scroll {
        Some(index as u16)
    } else if height > 0 && index >= scroll + height {
        Some((index + 1 - height) as u16)
    } else {
        None
    };
    if let Some(next) = next {
        match &mut app.loaded {
            Loaded::Show(show) => show.body.scroll = next,
            Loaded::Diff(diff) => diff.set_body_scroll(next),
        }
    }
}

// ---------------------------------------------------------------------------
// Scrolling bounds
// ---------------------------------------------------------------------------

fn page_step(app: &App) -> u16 {
    (app.size.height.saturating_sub(3) / 2).max(1)
}

fn max_scroll(app: &App) -> u16 {
    let count = match &app.loaded {
        Loaded::Show(show) => show.line_count(),
        Loaded::Diff(_) => app.derived.diff_rows.len(),
    };
    count.saturating_sub(1) as u16
}

fn max_hscroll(app: &App) -> u16 {
    match &app.loaded {
        Loaded::Show(show) => show.max_line_width().saturating_sub(1) as u16,
        Loaded::Diff(_) => 0,
    }
}

fn clamp_view(app: &mut App) {
    let scroll = max_scroll(app);
    let hscroll = max_hscroll(app);
    match &mut app.loaded {
        Loaded::Show(show) => {
            show.body.scroll = show.body.scroll.min(scroll);
            show.body.hscroll = show.body.hscroll.min(hscroll);
        }
        Loaded::Diff(diff) => {
            let current = diff.body_scroll();
            diff.set_body_scroll(current.min(scroll));
        }
    }
}

// ---------------------------------------------------------------------------
// Derived layout
// ---------------------------------------------------------------------------

fn body_is_visible(app: &App) -> bool {
    if app.size.width < SINGLE_PANE_MIN_WIDTH || app.size.height < MIN_HEIGHT {
        return false;
    }
    app.size.width >= SIDE_BY_SIDE_MIN_WIDTH || layout_focus(app) != Focus::Tree
}

/// Runs the selection-dependent work a frame needs, once per input batch.
pub(crate) fn settle(mut app: App) -> App {
    let theme = app.chrome.theme;
    let visible_body = body_is_visible(&app);
    match &mut app.loaded {
        Loaded::Show(show) => {
            if visible_body {
                show.ensure_highlight(&theme);
            }
            show.resync_search();
        }
        Loaded::Diff(diff) => {
            if visible_body {
                diff.ensure_highlight(&theme);
            }
            diff.resync_search();
        }
    }
    app.refresh_derived();
    app
}

impl App {
    fn refresh_derived(&mut self) {
        let key = DerivedKey {
            generation: self.generation,
            width: self.size.width,
            height: self.size.height,
            selected: self.loaded.selected().cloned(),
            tree_percent: self.chrome.tree_percent,
        };
        if self.derived.key.as_ref() == Some(&key) {
            return;
        }
        let diff_rows = self.compute_diff_rows();
        self.derived = Arc::new(Derived {
            key: Some(key),
            diff_rows,
        });
    }

    /// The wrapped visual rows of the active diff, recomputed without the cache.
    pub(crate) fn compute_diff_rows(&self) -> Vec<VisualRow> {
        let Loaded::Diff(diff) = &self.loaded else {
            return Vec::new();
        };
        let Some(active) = diff.active_diff() else {
            return Vec::new();
        };
        let (_, content, _) = frame_areas(self.size.width, self.size.height);
        let (old_width, new_width) = self.diff_side_widths(content, active);
        let empty = DiffHighlight::default();
        let highlights = diff.active_highlight().unwrap_or(&empty);
        layout_diff(
            active,
            old_width,
            new_width,
            &highlights.old,
            &highlights.new,
            &self.chrome.theme,
        )
    }

    fn diff_side_widths(&self, content: Rect, diff: &FileDiff) -> (usize, usize) {
        let gutter = gutter_width(diff);
        if self.size.width >= SIDE_BY_SIDE_MIN_WIDTH {
            let rest = 100 - self.chrome.tree_percent;
            let side = rest / 2;
            let (columns, _) =
                split_with_dividers(content, &[self.chrome.tree_percent, side, rest - side]);
            let old_inner =
                pane_block("", false, &self.chrome.theme, Edge::Middle).inner(columns[1]);
            let new_inner =
                pane_block("", false, &self.chrome.theme, Edge::Right).inner(columns[2]);
            (
                (old_inner.width as usize).saturating_sub(gutter),
                (new_inner.width as usize).saturating_sub(gutter),
            )
        } else {
            let inner = pane_block("", false, &self.chrome.theme, Edge::Solo).inner(content);
            let width = (inner.width as usize).saturating_sub(gutter);
            (width, width)
        }
    }
}

/// Replaces all step projections in one input batch with its final target. A
/// full reload wins when the view or endpoints also changed.
pub(crate) fn coalesce_commit_loads(app: &mut App, cmds: &mut Vec<Cmd>) {
    let Loaded::Diff(diff) = &mut app.loaded else {
        return;
    };
    let DiffViewState::Commits(commits) = &mut diff.view else {
        return;
    };
    if let Some(Cmd::Diff(_)) = cmds.iter().rev().find(|cmd| matches!(cmd, Cmd::Diff(_))) {
        return;
    }
    if let Some(index) = commits.picker.target()
        && let Some(step) = commits.picker.steps_slice().get(index).cloned()
    {
        diff.paging = Paging::Loading;
        cmds.push(Cmd::Step {
            request: diff_request(diff),
            index,
            step,
        });
    }
}

#[cfg(test)]
mod tests;
