//! The app shell and composer: the one place that owns shared state, routes
//! input to the component that owns it, and folds the component `OutMsg`s back
//! into shared state and effects.
//!
//! Nothing here performs I/O. `update` turns a message and the current model
//! into a replacement model plus a list of effects to run. Engine calls only
//! ever leave this module as a [`Cmd`]. The pure `view` lives in [`crate::view`].

use std::sync::Arc;

use base::{AreaSet, DiffRowKind, FileDiff, ProjectedFile, ProjectionMode, RepoPath, Selection};
use engine::{CommitStep, EngineError};
use ratatui::layout::{Position, Rect};

use crate::action::{Action, CommitsAction, GlobalAction, RangeAction, ShowAction};
use crate::commit_picker::{self, CommitPicker};
use crate::content::{
    self, ChangeKind, Commits, CommitsFocus, Diff, DiffBody, DiffHighlight, DiffPrompt, DiffSide,
    DiffViewState, Loaded, Paging, Range, RangeFocus, Scope, SearchMatch, SearchSide, Show,
    ShowBody, ShowFocus,
};
use crate::highlight;
use crate::icons::Icons;
use crate::input::{Key, Mouse, MouseKind};
use crate::overlay::{self, Overlay};
use crate::text_input::{Edit, TextInput};
use crate::theme::Theme;
use crate::tree::{self, RowKind, Tree};
use crate::view::diff::layout_diff;
use crate::view::geom::{
    Edge, PaneSlot, body_layout, commit_offset, frame_areas, gutter_width, pane_block,
    split_with_dividers, window_offset,
};

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

/// The kind of a rendered diff row: either an aligned diff row or a synthetic
/// hunk header inserted where context was collapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowKind {
    Diff(DiffRowKind),
    Hunk,
    /// A gap between hunks: a count of unchanged rows that were collapsed.
    Collapse(usize),
}

/// One visual row of a wrapped side-by-side diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub kind: VisualRowKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_runs: highlight::StyledLine,
    pub new_runs: highlight::StyledLine,
    pub continuation: bool,
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

    /// The change kind of a path in the active diff, for tree badges.
    pub(crate) fn change_kind(&self, path: &RepoPath) -> Option<ChangeKind> {
        self.loaded.change_kind(path)
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
mod tests {
    use super::*;
    use crate::content::{
        DiffSide, DiffViewState, Loaded, Search, SearchSide, ShowFocus, ShowHighlights,
    };
    use crate::icons::IconStyle;
    use crate::overlay::Overlay;
    use crate::theme::Theme;
    use crate::view::view;
    use base::{
        Area, AreaSet, ItemKind, ProjectedItem, RepoPath, Selection, SelectionGroup, SourceSpan,
        SupportedPath,
    };
    use ratatui::style::Color;

    fn item(text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: "item".to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: "item".to_owned(),
            span: SourceSpan::new(0, 0, 0, 0, 0, 0),
            canonical_text: text.to_owned(),
        }
    }

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path = SupportedPath::new(RepoPath::new(path).expect("valid path"))
            .expect("test path is supported");
        if text.is_empty() {
            return ProjectedFile::try_new(path, Vec::new()).expect("valid fixture");
        }
        ProjectedFile::try_new(path, vec![item(text)]).expect("valid fixture")
    }

    fn show_request() -> ShowRequest {
        ShowRequest {
            revision: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
        }
    }

    fn diff_request() -> DiffRequest {
        DiffRequest {
            base: "HEAD~1".to_owned(),
            target: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
            view: DiffView::Range,
        }
    }

    fn commits_request() -> DiffRequest {
        DiffRequest {
            view: DiffView::Commits,
            ..diff_request()
        }
    }

    fn file_diff(path: &str, old: Option<&str>, new: Option<&str>) -> FileDiff {
        match (old, new) {
            (None, Some(text)) => FileDiff::Added {
                new: projected(path, text),
            },
            (Some(text), None) => FileDiff::Deleted {
                old: projected(path, text),
            },
            (Some(old_text), Some(new_text)) => FileDiff::Modified {
                old: projected(path, old_text),
                new: projected(path, new_text),
            },
            (None, None) => panic!("a test diff needs at least one side"),
        }
    }

    fn base_app(request: LoadRequest) -> App {
        App::new(
            "/repo".to_owned(),
            request,
            "all".to_owned(),
            100,
            30,
            Theme::dark(),
            Icons::new(IconStyle::None),
        )
    }

    fn show_app(files: Vec<ProjectedFile>) -> App {
        let app = base_app(LoadRequest::Show {
            revision: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
        });
        let (next, _) = update(
            Msg::ShowLoaded {
                request: show_request(),
                result: Ok(files.into()),
            },
            &app,
        );
        settle(next)
    }

    fn diff_app(diffs: Vec<FileDiff>) -> App {
        let app = base_app(LoadRequest::Diff {
            base: "HEAD~1".to_owned(),
            target: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
            view: DiffView::Range,
        });
        let (next, _) = update(
            Msg::DiffLoaded {
                request: diff_request(),
                result: Ok(diffs.into()),
            },
            &app,
        );
        settle(next)
    }

    fn commit_steps(count: usize) -> Vec<CommitStep> {
        use git::{HashKind, ObjectId};
        (0..count)
            .map(|index| CommitStep {
                parent_id: ObjectId {
                    kind: HashKind::Sha1,
                    bytes: vec![index as u8; 20],
                },
                commit_id: ObjectId {
                    kind: HashKind::Sha1,
                    bytes: vec![(index + 1) as u8; 20],
                },
                subject: format!("Commit subject {index}"),
            })
            .collect()
    }

    fn commits_app(count: usize) -> App {
        let app = base_app(LoadRequest::Diff {
            base: "HEAD~1".to_owned(),
            target: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
            view: DiffView::Commits,
        });
        let (next, _) = update(
            Msg::HistoryLoaded {
                request: commits_request(),
                result: Ok((
                    commit_steps(count).into(),
                    vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into(),
                )),
            },
            &app,
        );
        settle(next)
    }

    fn show_ref(app: &App) -> &Show {
        match &app.loaded {
            Loaded::Show(show) => show,
            _ => panic!("expected a show"),
        }
    }

    fn diff_ref(app: &App) -> &Diff {
        match &app.loaded {
            Loaded::Diff(diff) => diff,
            _ => panic!("expected a diff"),
        }
    }

    fn commits_ref(app: &App) -> &crate::commit_picker::CommitPicker {
        match &app.loaded {
            Loaded::Diff(diff) => match &diff.view {
                DiffViewState::Commits(commits) => &commits.picker,
                _ => panic!("expected commits"),
            },
            _ => panic!("expected a diff"),
        }
    }

    fn selected(app: &App) -> Option<String> {
        app.loaded.selected().map(ToString::to_string)
    }

    fn diff_focus_of(app: &App) -> Focus {
        diff_focus(diff_ref(app))
    }

    fn focus_content(app: &mut App) {
        match &mut app.loaded {
            Loaded::Show(show) => show.focus = ShowFocus::Body,
            Loaded::Diff(diff) => match &mut diff.view {
                DiffViewState::Range(range) => range.focus = RangeFocus::Diff,
                DiffViewState::Commits(commits) => commits.focus = CommitsFocus::Diff,
            },
        }
    }

    fn two_files() -> App {
        show_app(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")])
    }

    // -----------------------------------------------------------------------
    // Install and selection
    // -----------------------------------------------------------------------

    #[test]
    fn a_loaded_result_installs_files_and_preserves_the_selection() {
        let app = two_files();
        let (next, _) = update(
            Msg::ShowLoaded {
                request: ShowRequest {
                    mode: ProjectionMode::Signatures,
                    ..show_request()
                },
                result: Ok(vec![projected("a.rs", "A\n"), projected("b.rs", "B\n")].into()),
            },
            &app,
        );

        assert_eq!(next.loaded.mode(), ProjectionMode::Signatures);
        assert_eq!(selected(&next).as_deref(), Some("a.rs"));
        assert_eq!(show_ref(&next).active_text(), Some("A\n"));
    }

    #[test]
    fn a_reload_that_drops_the_selected_file_picks_the_nearest_visible_one() {
        let mut app = show_app(vec![
            projected("a.rs", "a\n"),
            projected("b.rs", "b\n"),
            projected("c.rs", "c\n"),
        ]);
        app.tree.cursor = app
            .tree
            .row_of_file(&RepoPath::new("b.rs").unwrap())
            .unwrap();
        app.loaded
            .set_selected(Some(RepoPath::new("b.rs").unwrap()));

        let (next, _) = update(
            Msg::ShowLoaded {
                request: show_request(),
                result: Ok(vec![projected("a.rs", "a\n"), projected("c.rs", "c\n")].into()),
            },
            &app,
        );
        assert_eq!(selected(&next).as_deref(), Some("c.rs"));
    }

    #[test]
    fn a_failed_reload_keeps_the_last_model_and_shows_a_diagnostic() {
        let app = two_files();
        let error = EngineError::Selection(base::SelectionError::UnknownArea {
            name: "x".to_owned(),
        });
        let (next, _) = update(
            Msg::ShowLoaded {
                request: ShowRequest {
                    mode: ProjectionMode::Signatures,
                    ..show_request()
                },
                result: Err(Box::new(error)),
            },
            &app,
        );

        assert_eq!(
            next.loaded.mode(),
            ProjectionMode::Types,
            "mode must not change"
        );
        assert_eq!(show_ref(&next).files.len(), 2);
        assert!(next.diagnostic.is_some());
        assert!(next.loaded.selected().is_some());
    }

    #[test]
    fn empty_projections_are_hidden_from_the_tree() {
        let app = show_app(vec![projected("empty.rs", ""), projected("full.rs", "x\n")]);
        let labels: Vec<&str> = app.tree.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["full.rs"]);
        assert!(
            !app.loaded
                .visible()
                .contains(&RepoPath::new("empty.rs").unwrap())
        );
    }

    #[test]
    fn a_diff_tree_contains_only_changed_paths() {
        let app = diff_app(vec![
            file_diff("src/a.rs", Some("a\n"), Some("A\n")),
            file_diff("src/b.rs", None, Some("b\n")),
        ]);
        let labels: Vec<&str> = app.tree.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["src", "a.rs", "b.rs"]);
    }

    #[test]
    fn tree_rows_follow_raw_path_order_and_are_nested() {
        let app = show_app(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
            projected("src/main.rs", "m\n"),
        ]);
        let labels: Vec<&str> = app.tree.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
        assert_eq!(app.tree.rows[0].depth, 0);
        assert_eq!(app.tree.rows[1].depth, 0);
        assert_eq!(app.tree.rows[2].depth, 1);
    }

    #[test]
    fn collapsing_a_directory_hides_its_children_and_expanding_restores_them() {
        let mut app = show_app(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
            projected("src/main.rs", "m\n"),
        ]);
        app.tree.cursor = 1;
        assert!(matches!(
            app.tree.current_row().unwrap().kind,
            RowKind::Directory { .. }
        ));

        let (collapsed, _) = update(Msg::Key(Key::Enter), &app);
        let labels: Vec<&str> = collapsed
            .tree
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(labels, vec!["a.rs", "src"]);
        assert!(matches!(
            collapsed.tree.rows[1].kind,
            RowKind::Directory {
                expanded: false,
                ..
            }
        ));

        let (expanded, _) = update(Msg::Key(Key::Right), &collapsed);
        let labels: Vec<&str> = expanded
            .tree
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
    }

    #[test]
    fn left_on_a_directory_row_folds_it() {
        let mut app = show_app(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
        ]);
        app.tree.cursor = 1;
        let (folded, _) = update(Msg::Key(Key::Left), &app);
        assert!(matches!(
            folded.tree.rows[1].kind,
            RowKind::Directory {
                expanded: false,
                ..
            }
        ));
    }

    // -----------------------------------------------------------------------
    // Commits
    // -----------------------------------------------------------------------

    #[test]
    fn empty_commit_history_installs_a_diff_and_focuses_commits() {
        let app = base_app(LoadRequest::Diff {
            base: "HEAD~1".to_owned(),
            target: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
            view: DiffView::Commits,
        });
        let (next, commands) = update(
            Msg::HistoryLoaded {
                request: commits_request(),
                result: Ok((Vec::new().into(), Vec::new().into())),
            },
            &app,
        );
        assert!(commands.is_empty());
        assert_eq!(next.loaded.mode(), ProjectionMode::Types);
        assert!(commits_ref(&next).is_empty());
        assert!(next.loaded.visible().is_empty());
        assert!(next.loaded.selected().is_none());
        let (unchanged, commands) = update(Msg::Key(Key::Down), &next);
        assert!(commands.is_empty());
        assert_eq!(commits_ref(&unchanged).cursor(), 0);
    }

    #[test]
    fn commit_picker_renders_rows_and_selected_revision_labels() {
        let app = commits_app(12);
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("Commits"), "{text}");
        assert!(text.contains("Files"), "{text}");
        assert!(text.contains("Commit sub"), "{text}");
        assert!(text.contains("1/12"), "{text}");
        assert!(text.contains("0000000..0101010"), "{text}");

        let mut empty = commits_app(0);
        let text = buffer_text(&render(&empty, 100, 20));
        assert!(text.contains("No commits"), "{text}");
        let _ = &mut empty;
    }

    #[test]
    fn commit_picker_mouse_uses_drawn_geometry_and_loads_selected_step() {
        let app = commits_app(12);
        let (mut clicked, mut commands) = update(Msg::Mouse(click(4, 3)), &app);
        coalesce_commit_loads(&mut clicked, &mut commands);
        assert_eq!(diff_focus_of(&clicked), Focus::Commits);
        assert!(matches!(commands.as_slice(), [Cmd::Step { index: 1, .. }]));

        let (mut wheeled, mut commands) = update(Msg::Mouse(scroll(4, 3, false)), &app);
        coalesce_commit_loads(&mut wheeled, &mut commands);
        assert_eq!(diff_focus_of(&wheeled), Focus::Commits);
        assert!(matches!(commands.as_slice(), [Cmd::Step { index: 3, .. }]));

        let mut narrow = app.clone();
        narrow.size = Size {
            width: 60,
            height: 30,
        };
        let (mut narrow_clicked, mut commands) = update(Msg::Mouse(click(4, 3)), &narrow);
        coalesce_commit_loads(&mut narrow_clicked, &mut commands);
        assert!(matches!(commands.as_slice(), [Cmd::Step { index: 1, .. }]));

        let (files, commands) = update(Msg::Mouse(click(4, 11)), &app);
        assert_eq!(diff_focus_of(&files), Focus::Tree);
        assert!(commands.is_empty());
    }

    #[test]
    fn batched_commit_navigation_loads_only_the_final_step_and_failure_keeps_the_displayed_step() {
        let mut app = commits_app(6);
        let displayed = diff_ref(&app).diffs().to_vec();
        let displayed_labels = diff_ref(&app).revisions();
        let mut commands = Vec::new();
        for key in [Key::Down, Key::Char('j'), Key::Down] {
            let (next, produced) = update(Msg::Key(key), &app);
            app = next;
            commands.extend(produced);
        }
        assert_eq!(
            commits_ref(&app).cursor(),
            0,
            "the shown step is still loaded"
        );
        assert_eq!(commits_ref(&app).target(), Some(3));
        assert!(commands.is_empty(), "a step load is deferred to coalescing");
        coalesce_commit_loads(&mut app, &mut commands);
        assert!(matches!(commands.as_slice(), [Cmd::Step { index: 3, .. }]));
        assert_eq!(diff_ref(&app).diffs(), displayed.as_slice());
        assert_eq!(diff_ref(&app).revisions(), displayed_labels);

        let (loaded, _) = update(
            Msg::StepLoaded {
                index: 3,
                result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("new\n"))].into()),
            },
            &app,
        );
        assert_eq!(commits_ref(&loaded).cursor(), 3);
        assert_eq!(commits_ref(&loaded).target(), None);
        assert_ne!(diff_ref(&loaded).revisions(), displayed_labels);
        assert_eq!(loaded.loaded.selected(), app.loaded.selected());

        let (failed, commands) = update(
            Msg::StepLoaded {
                index: 3,
                result: Err(Box::new(EngineError::Selection(
                    base::SelectionError::UnknownArea {
                        name: "missing".to_owned(),
                    },
                ))),
            },
            &app,
        );
        assert!(commands.is_empty());
        assert_eq!(commits_ref(&failed).cursor(), 0);
        assert_eq!(commits_ref(&failed).target(), None);
        assert_eq!(diff_ref(&failed).diffs(), displayed.as_slice());
        assert_eq!(diff_ref(&failed).revisions(), displayed_labels);
        assert!(failed.diagnostic.is_some());
    }

    #[test]
    fn commit_navigation_back_to_loaded_step_cancels_batched_projection() {
        let mut app = commits_app(3);
        let mut commands = Vec::new();
        for key in [Key::Down, Key::Up] {
            let (next, produced) = update(Msg::Key(key), &app);
            app = next;
            commands.extend(produced);
        }
        coalesce_commit_loads(&mut app, &mut commands);
        assert!(commands.is_empty());
        assert!(!app.is_busy());
        assert_eq!(commits_ref(&app).target(), None);
    }

    #[test]
    fn palette_switches_between_range_and_commit_views() {
        let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        assert!(
            app.loaded
                .palette_entries()
                .iter()
                .any(|(action, ..)| *action == Action::Range(RangeAction::SwitchToCommits))
        );
        let mut next = app.clone();
        let mut commands = Vec::new();
        apply_action(
            &mut next,
            Action::Range(RangeAction::SwitchToCommits),
            &mut commands,
        );
        assert_eq!(
            commands,
            vec![Cmd::Diff(DiffRequest {
                view: DiffView::Commits,
                ..diff_request()
            })]
        );
        assert_eq!(diff_ref(&next).revisions(), diff_ref(&app).revisions());
    }

    // -----------------------------------------------------------------------
    // Settle deferral
    // -----------------------------------------------------------------------

    #[test]
    fn navigation_defers_highlighting_until_settle() {
        let app = show_app(vec![
            projected("a.rs", "pub fn a();\n"),
            projected("b.rs", "pub fn b();\n"),
        ]);
        // Force a fresh selection that has not been highlighted.
        let mut fresh = app.clone();
        if let Loaded::Show(show) = &mut fresh.loaded {
            show.highlight = ShowHighlights::default();
        }
        assert!(show_ref(&fresh).active_lines().is_none());

        let (moved, _) = update(Msg::Key(Key::Down), &fresh);
        assert!(
            show_ref(&moved).active_lines().is_none(),
            "update must not highlight"
        );

        let settled = settle(moved);
        assert!(show_ref(&settled).active_lines().is_some());
    }

    #[test]
    fn settle_skips_highlighting_when_the_body_is_hidden() {
        let mut app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
        app.size = Size {
            width: 60,
            height: 30,
        };
        app.tree.cursor = 0;
        if let Loaded::Show(show) = &mut app.loaded {
            show.focus = ShowFocus::Tree;
            show.highlight = ShowHighlights::default();
        }
        let settled = settle(app);
        assert!(show_ref(&settled).active_lines().is_none());
    }

    #[test]
    fn the_highlight_cache_is_bounded() {
        let mut cache: crate::cache::BoundedCache<usize> = crate::cache::BoundedCache::default();
        for index in 0..(crate::cache::CAP + 10) {
            let path = RepoPath::new(format!("f{index}.rs")).expect("valid path");
            cache.insert(path, index);
        }
        assert!(!cache.contains_key(&RepoPath::new("f0.rs").expect("valid path")));
        let newest = RepoPath::new(format!("f{}.rs", crate::cache::CAP + 9)).expect("valid path");
        assert!(cache.contains_key(&newest));
    }

    // -----------------------------------------------------------------------
    // Mode and overlays
    // -----------------------------------------------------------------------

    #[test]
    fn mode_picker_defers_the_mode_change_until_selection() {
        let app = two_files();
        let (opened, cmds) = update(Msg::Key(Key::Char('m')), &app);
        assert!(cmds.is_empty());
        assert!(matches!(opened.overlay, Some(Overlay::Mode { .. })));

        let (down, _) = update(Msg::Key(Key::Down), &opened);
        let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);

        assert_eq!(app.loaded.mode(), ProjectionMode::Types);
        assert_eq!(
            chosen.loaded.mode(),
            ProjectionMode::Types,
            "mode changes on load"
        );
        assert_eq!(chosen.overlay, None);
        assert_eq!(
            cmds,
            vec![Cmd::Show(ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            })]
        );
    }

    #[test]
    fn mode_picker_on_a_diff_keeps_the_diff_shape() {
        let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let (opened, _) = update(Msg::Key(Key::Char('m')), &app);
        let (down, _) = update(Msg::Key(Key::Down), &opened);
        let (_, cmds) = update(Msg::Key(Key::Enter), &down);
        assert_eq!(
            cmds,
            vec![Cmd::Diff(DiffRequest {
                mode: ProjectionMode::Signatures,
                ..diff_request()
            })]
        );
    }

    #[test]
    fn the_mode_picker_lists_the_modes() {
        let mut app = two_files();
        app.overlay = Some(Overlay::mode(ProjectionMode::Types));
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("types"), "{text}");
        assert!(text.contains("signatures"), "{text}");
    }

    #[test]
    fn tree_resize_keys_step_clamp_and_reset() {
        let app = two_files();
        let (grow, _) = update(Msg::Key(Key::Char(']')), &app);
        assert_eq!(grow.chrome.tree_percent, TREE_DEFAULT_PERCENT + TREE_STEP);
        let (shrink, _) = update(Msg::Key(Key::Char('[')), &grow);
        assert_eq!(shrink.chrome.tree_percent, TREE_DEFAULT_PERCENT);

        let mut widest = app.clone();
        widest.chrome.tree_percent = TREE_MAX_PERCENT;
        let (widest, _) = update(Msg::Key(Key::Char(']')), &widest);
        assert_eq!(widest.chrome.tree_percent, TREE_MAX_PERCENT);

        let mut narrowest = app.clone();
        narrowest.chrome.tree_percent = TREE_MIN_PERCENT;
        let (narrowest, _) = update(Msg::Key(Key::Char('[')), &narrowest);
        assert_eq!(narrowest.chrome.tree_percent, TREE_MIN_PERCENT);

        let (reset, _) = update(Msg::Key(Key::Char('\\')), &grow);
        assert_eq!(reset.chrome.tree_percent, TREE_DEFAULT_PERCENT);
    }

    #[test]
    fn a_wider_tree_still_leaves_a_usable_body() {
        let mut app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
        app.chrome.tree_percent = TREE_MAX_PERCENT;
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("Files"), "{text}");
        assert!(text.contains("pub fn a();"), "{text}");
    }

    #[test]
    fn tab_toggles_the_tree_and_both_diff_panes() {
        let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let (step, _) = update(Msg::Key(Key::Tab), &app);
        assert!(diff_ref(&step).focus_is_diff());
        let (step, _) = update(Msg::Key(Key::Tab), &step);
        assert_eq!(diff_focus_of(&step), Focus::Tree);
        let (step, _) = update(Msg::Key(Key::BackTab), &step);
        assert!(diff_ref(&step).focus_is_diff());

        let app = two_files();
        let (step, _) = update(Msg::Key(Key::Tab), &app);
        assert_eq!(show_ref(&step).focus, ShowFocus::Body);
        let (step, _) = update(Msg::Key(Key::BackTab), &step);
        assert_eq!(show_ref(&step).focus, ShowFocus::Tree);
    }

    #[test]
    fn help_toggles_and_swallows_navigation() {
        let app = two_files();
        let (help, _) = update(Msg::Key(Key::Char('?')), &app);
        assert_eq!(help.overlay, Some(Overlay::Help));

        let (still, _) = update(Msg::Key(Key::Char('j')), &help);
        assert_eq!(still.tree.cursor, help.tree.cursor);
        assert_eq!(still.overlay, Some(Overlay::Help));

        let (closed, _) = update(Msg::Key(Key::Esc), &help);
        assert_eq!(closed.overlay, None);
    }

    #[test]
    fn revision_prompt_prefills_the_current_revision() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
        let prompt = show_ref(&opened).prompt.as_ref().expect("prompt");
        assert_eq!(prompt.text, "HEAD");
        assert_eq!(prompt.cursor, 4);
    }

    #[test]
    fn confirming_a_revision_emits_a_load_with_that_revision() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
        let mut state = opened;
        for _ in 0.."HEAD".len() {
            let (next, _) = update(Msg::Key(Key::Backspace), &state);
            state = next;
        }
        for character in "HEAD~2".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &state);
            state = next;
        }
        let (applied, cmds) = update(Msg::Key(Key::Enter), &state);
        assert!(show_ref(&applied).prompt.is_none());
        assert_eq!(
            cmds,
            vec![Cmd::Show(ShowRequest {
                revision: "HEAD~2".to_owned(),
                ..show_request()
            })]
        );
    }

    #[test]
    fn escaping_a_revision_prompt_emits_nothing() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
        let (closed, cmds) = update(Msg::Key(Key::Esc), &opened);
        assert!(show_ref(&closed).prompt.is_none());
        assert!(cmds.is_empty());
    }

    #[test]
    fn diff_revision_keys_target_base_and_target_fields() {
        let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("b\n"))]);
        let (base, _) = update(Msg::Key(Key::Char('b')), &app);
        assert_eq!(
            diff_ref(&base).prompt.as_ref().map(|p| p.side),
            Some(DiffSide::Base)
        );
        let (target, _) = update(Msg::Key(Key::Char('t')), &app);
        assert_eq!(
            diff_ref(&target).prompt.as_ref().map(|p| p.side),
            Some(DiffSide::Target)
        );
    }

    #[test]
    fn the_revision_prompt_renders_its_label_and_text() {
        let mut app = two_files();
        if let Loaded::Show(show) = &mut app.loaded {
            show.prompt = Some(TextInput::new("HEAD~2"));
        }
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("revision"), "{text}");
        assert!(text.contains("HEAD~2"), "{text}");
    }

    fn area_set() -> AreaSet {
        AreaSet::new([
            Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
            Area::new("web", [RepoPath::new("src/web").unwrap()]).unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn scope_key_opens_the_chooser_and_requests_areas() {
        let app = two_files();
        let (opened, cmds) = update(Msg::Key(Key::Char('s')), &app);
        assert!(matches!(opened.overlay, Some(Overlay::Scope(_))));
        assert_eq!(cmds, vec![Cmd::LoadAreas]);
    }

    #[test]
    fn choosing_all_from_the_chooser_loads_everything() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
        let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
        let (chosen, cmds) = update(Msg::Key(Key::Enter), &loaded);
        assert_eq!(chosen.overlay, None);
        assert_eq!(
            cmds,
            vec![Cmd::Show(ShowRequest {
                selection: Selection::all(),
                ..show_request()
            })]
        );
    }

    #[test]
    fn selecting_an_area_loads_that_area() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
        let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
        let (down, _) = update(Msg::Key(Key::Down), &loaded);
        let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);
        assert_eq!(chosen.overlay, None);
        let expected = Selection::new(vec![SelectionGroup::Area(
            Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
        )]);
        assert_eq!(
            cmds,
            vec![Cmd::Show(ShowRequest {
                selection: expected,
                ..show_request()
            })]
        );
    }

    #[test]
    fn a_config_error_keeps_the_chooser_open_with_a_message() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
        let error = EngineError::Selection(base::SelectionError::UnknownArea {
            name: "x".to_owned(),
        });
        let (loaded, _) = update(Msg::AreasLoaded(Err(Box::new(error))), &opened);
        match loaded.overlay {
            Some(Overlay::Scope(chooser)) => assert!(chooser.error.is_some()),
            other => panic!("expected a scope chooser, got {other:?}"),
        }
    }

    #[test]
    fn literal_path_entry_builds_a_path_selection() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
        let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
        let mut state = loaded;
        for _ in 0..3 {
            let (next, _) = update(Msg::Key(Key::Down), &state);
            state = next;
        }
        let (input, _) = update(Msg::Key(Key::Enter), &state);
        let mut typed = input;
        for character in "src/lib.rs".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &typed);
            typed = next;
        }
        let (chosen, cmds) = update(Msg::Key(Key::Enter), &typed);
        assert_eq!(chosen.overlay, None);
        let expected = Selection::new(vec![SelectionGroup::Path(
            RepoPath::new("src/lib.rs").unwrap(),
        )]);
        assert_eq!(
            cmds,
            vec![Cmd::Show(ShowRequest {
                selection: expected,
                ..show_request()
            })]
        );
    }

    #[test]
    fn search_finds_matches_and_steps_them() {
        let app = show_app(vec![projected("a.rs", "alpha\nbeta\nalpha again\n")]);
        let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
        assert!(matches!(opened.overlay, Some(Overlay::Search(_))));

        let mut state = opened;
        for character in "alpha".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &state);
            state = next;
        }
        assert_eq!(state.loaded.search().expect("live search").matches.len(), 2);

        let (committed, _) = update(Msg::Key(Key::Enter), &state);
        assert_eq!(committed.overlay, None);
        let (stepped, _) = update(Msg::Key(Key::Char('n')), &committed);
        assert_eq!(stepped.loaded.search().unwrap().cursor, 1);
        let (back, _) = update(Msg::Key(Key::Char('N')), &stepped);
        assert_eq!(back.loaded.search().unwrap().cursor, 0);
    }

    #[test]
    fn switching_files_recomputes_a_committed_search() {
        let mut app = show_app(vec![
            projected("a.rs", "alpha\n"),
            projected("b.rs", "beta\nalpha\n"),
        ]);
        if let Loaded::Show(show) = &mut app.loaded {
            show.search = Some(Search {
                needle: "alpha".to_owned(),
                matches: vec![SearchMatch {
                    side: SearchSide::Show,
                    line: 1,
                    start: 0,
                    end: 5,
                }],
                cursor: 0,
            });
        }
        app.loaded
            .set_selected(Some(RepoPath::new("b.rs").unwrap()));
        if let Loaded::Show(show) = &mut app.loaded {
            show.resync_search();
        }
        let search = app.loaded.search().expect("search");
        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.matches[0].line, 2, "the match moved to the new file");
    }

    #[test]
    fn q_types_into_a_search_input_instead_of_quitting() {
        let app = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
        let (typed, _) = update(Msg::Key(Key::Char('q')), &opened);
        assert!(!typed.chrome.quit);
        match &typed.overlay {
            Some(Overlay::Search(state)) => assert_eq!(state.input.text, "q"),
            other => panic!("expected a search overlay, got {other:?}"),
        }
    }

    #[test]
    fn the_palette_and_finder_render_their_lists() {
        let mut app = two_files();
        app.overlay = Some(Overlay::palette(&overlay::Ctx {
            visible: app.loaded.visible(),
            entries: &app.loaded.palette_entries(),
            mode: app.loaded.mode(),
        }));
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("commands"), "{text}");
        assert!(text.contains("Switch Types"), "{text}");

        let mut app = two_files();
        app.overlay = Some(Overlay::finder(&overlay::Ctx {
            visible: app.loaded.visible(),
            entries: &app.loaded.palette_entries(),
            mode: app.loaded.mode(),
        }));
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("find file"), "{text}");
        assert!(text.contains("a.rs"), "{text}");
    }

    // -----------------------------------------------------------------------
    // Scrolling and mouse
    // -----------------------------------------------------------------------

    fn click(column: u16, row: u16) -> Mouse {
        Mouse {
            column,
            row,
            kind: MouseKind::Click,
        }
    }

    fn scroll(column: u16, row: u16, up: bool) -> Mouse {
        Mouse {
            column,
            row,
            kind: if up {
                MouseKind::ScrollUp
            } else {
                MouseKind::ScrollDown
            },
        }
    }

    const TREE_X: u16 = 2;
    const TREE_Y: u16 = 2;
    const CONTENT_X: u16 = 50;

    #[test]
    fn page_keys_move_the_cursor_in_the_tree_and_scroll_the_body() {
        let app = show_app(vec![
            projected("a.rs", "one\ntwo\nthree\nfour\nfive\n"),
            projected("b.rs", "x\n"),
        ]);
        let (down, _) = update(Msg::Key(Key::PageDown), &app);
        assert!(down.tree.cursor > app.tree.cursor);

        let mut body = app.clone();
        focus_content(&mut body);
        let (scrolled, _) = update(Msg::Key(Key::PageDown), &body);
        assert!(show_ref(&scrolled).body.scroll > 0);
    }

    #[test]
    fn body_scrolling_is_clamped_to_the_content() {
        let mut app = show_app(vec![projected("a.rs", "one\ntwo\nthree\n")]);
        focus_content(&mut app);
        for _ in 0..10 {
            let (next, _) = update(Msg::Key(Key::Char('j')), &app);
            app = next;
        }
        assert_eq!(show_ref(&app).body.scroll, 2);
        for _ in 0..10 {
            let (next, _) = update(Msg::Key(Key::Char('k')), &app);
            app = next;
        }
        assert_eq!(show_ref(&app).body.scroll, 0);
    }

    #[test]
    fn clicking_a_file_selects_it() {
        let app = two_files();
        let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
        assert_eq!(selected(&next).as_deref(), Some("b.rs"));
        assert_eq!(next.tree.cursor, 1);
        assert!(matches!(next.loaded, Loaded::Show(ref show) if show.focus == ShowFocus::Tree));
    }

    #[test]
    fn clicking_a_directory_folds_it() {
        let app = show_app(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
        ]);
        let (folded, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
        let labels: Vec<&str> = folded
            .tree
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(labels, vec!["a.rs", "src"]);
    }

    #[test]
    fn the_wheel_moves_the_tree_cursor_and_selects_as_it_passes() {
        let app = show_app(vec![
            projected("a.rs", "a\n"),
            projected("b.rs", "b\n"),
            projected("c.rs", "c\n"),
        ]);
        let (moved, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &app);
        assert_eq!(
            moved.tree.cursor, 2,
            "the wheel step clamps to the last row"
        );
        assert_eq!(selected(&moved).as_deref(), Some("c.rs"));
    }

    #[test]
    fn the_wheel_scrolls_the_content_and_focuses_it() {
        let app = show_app(vec![projected("a.rs", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")]);
        let (down, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &app);
        assert!(matches!(down.loaded, Loaded::Show(ref show) if show.focus == ShowFocus::Body));
        assert_eq!(show_ref(&down).body.scroll, 3);
        let (up, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, true)), &down);
        assert_eq!(show_ref(&up).body.scroll, 0);
    }

    #[test]
    fn the_wheel_scrolls_a_diff_and_clicking_it_focuses() {
        let app = diff_app(vec![file_diff(
            "a.rs",
            Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"),
            Some("1\n2\n3\n4\n5\n6\n7\n8\n9\nX\n"),
        )]);
        let (scrolled, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &app);
        assert!(diff_ref(&scrolled).focus_is_diff());
        assert!(diff_ref(&scrolled).body_scroll() > 0);

        let (clicked, _) = update(Msg::Mouse(click(CONTENT_X, 5)), &app);
        assert!(diff_ref(&clicked).focus_is_diff());
        assert_eq!(diff_ref(&clicked).body_scroll(), 0);
    }

    #[test]
    fn mouse_is_ignored_while_an_overlay_is_open() {
        let mut app = two_files();
        app.overlay = Some(Overlay::Help);
        let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
        assert_eq!(next.tree.cursor, app.tree.cursor);
        assert_eq!(next.loaded.selected(), app.loaded.selected());
    }

    #[test]
    fn a_click_on_the_header_or_a_divider_does_nothing() {
        let app = two_files();
        let (header, _) = update(Msg::Mouse(click(5, 0)), &app);
        assert_eq!(header.tree.cursor, app.tree.cursor);

        let mut body_focused = app.clone();
        focus_content(&mut body_focused);
        let (divider, _) = update(Msg::Mouse(click(29, 5)), &body_focused);
        assert!(matches!(divider.loaded, Loaded::Show(ref show) if show.focus == ShowFocus::Body));
        assert_eq!(divider.loaded.selected(), app.loaded.selected());
    }

    // -----------------------------------------------------------------------
    // Status, diagnostic, spinner
    // -----------------------------------------------------------------------

    #[test]
    fn the_status_bar_shows_mode_scope_and_hints() {
        let app = show_app(vec![projected("a.rs", "x\n")]);
        let buffer = render(&app, 120, 20);
        let header = buffer_row(&buffer, 0);
        let footer = buffer_row(&buffer, 19);
        assert!(header.contains("types"), "{header}");
        assert!(header.contains("HEAD"), "{header}");
        assert!(header.contains("scope:"), "{header}");
        assert!(!header.contains("a.rs"), "{header}");
        assert!(footer.contains("a.rs"), "{footer}");
        assert!(footer.contains("1 lines"), "{footer}");
        assert!(footer.contains("help"), "{footer}");
    }

    #[test]
    fn narrow_chrome_keeps_context_and_the_selected_path() {
        let app = show_app(vec![projected("src/very/deep/file.rs", "x\n")]);
        let buffer = render(&app, 60, 20);
        let header = buffer_row(&buffer, 0);
        let footer = buffer_row(&buffer, 19);
        assert!(header.contains("types"), "{header}");
        assert!(header.contains("HEAD"), "{header}");
        assert!(footer.contains("file.rs"), "{footer}");
        assert!(footer.contains("lines"), "{footer}");
        assert!(footer.contains("help"), "{footer}");
        assert_eq!(
            buffer_text(&buffer)
                .matches("src/very/deep/file.rs")
                .count(),
            1
        );
    }

    #[test]
    fn the_status_bar_counts_diff_changes() {
        let app = diff_app(vec![file_diff(
            "a.rs",
            Some("one\ntwo\n"),
            Some("one\n2\n"),
        )]);
        let text = buffer_text(&render(&app, 120, 20));
        assert!(text.contains("+1"), "{text}");
        assert!(text.contains("−1"), "{text}");
    }

    #[test]
    fn a_diagnostic_renders_as_a_toast() {
        let mut app = two_files();
        app.diagnostic = Some(Diagnostic::new("boom".to_owned()));
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn a_load_marks_the_model_busy_until_it_completes() {
        let app = two_files();
        assert!(!app.is_busy(), "an installed model is idle");

        let (opened, cmds) = update(Msg::Key(Key::Char('m')), &app);
        assert!(cmds.is_empty(), "opening the picker emits no effect");
        let (down, _) = update(Msg::Key(Key::Down), &opened);
        let (loading, cmds) = update(Msg::Key(Key::Enter), &down);
        assert!(!cmds.is_empty());
        assert!(loading.is_busy(), "an emitted load is busy");

        let (done, _) = update(
            Msg::ShowLoaded {
                request: ShowRequest {
                    mode: ProjectionMode::Signatures,
                    ..show_request()
                },
                result: Ok(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")].into()),
            },
            &loading,
        );
        assert!(!done.is_busy(), "completion clears the busy state");
    }

    #[test]
    fn a_tick_advances_the_spinner_and_expires_a_diagnostic() {
        let mut app = two_files();
        app.diagnostic = Some(Diagnostic::new("boom".to_owned()));
        let before = app.chrome.spinner;
        let (next, _) = update(Msg::Tick, &app);
        assert_eq!(next.chrome.spinner, before.wrapping_add(1));

        for _ in 0..DIAGNOSTIC_TICKS {
            let (ticked, _) = update(Msg::Tick, &app);
            app = ticked;
        }
        assert!(app.diagnostic.is_none());
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn render(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| view(app, frame)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }
        text
    }

    fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .filter_map(|x| buffer.cell((x, y)))
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn a_wide_terminal_shows_the_tree_and_the_projection() {
        let app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("Files"), "tree pane missing: {text}");
        assert!(text.contains("pub fn a();"), "body missing: {text}");
    }

    #[test]
    fn the_tree_keeps_a_quiet_selected_row_when_the_body_has_focus() {
        use crate::theme::{Capability, Flavor};

        for flavor in [Flavor::Dark, Flavor::Light] {
            let mut app = two_files();
            app.chrome.theme = Theme::new(flavor, Capability::TrueColor);

            let focused = render(&app, 100, 20);
            let focused_row = (0..focused.area.height)
                .find(|&y| {
                    focused
                        .cell((2, y))
                        .is_some_and(|cell| cell.symbol() == "▌")
                })
                .expect("selected file row");
            assert_eq!(focused.cell((2, focused_row)).unwrap().symbol(), "▌");

            focus_content(&mut app);
            let unfocused = render(&app, 100, 20);
            assert_eq!(unfocused.cell((2, focused_row)).unwrap().symbol(), "▏");
        }

        let mut app = two_files();
        app.chrome.theme = Theme::new(Flavor::Dark, Capability::NoColor);
        focus_content(&mut app);
        assert!(buffer_text(&render(&app, 100, 20)).contains("▏a.rs"));
    }

    #[test]
    fn nerd_icons_render_and_leave_labels_intact() {
        let build = || {
            show_app(vec![
                projected("src/main.rs", "x\n"),
                projected("src/Main.elm", "y\n"),
            ])
        };
        let plain = build();
        let mut nerd = build();
        nerd.chrome.icons = Icons::new(IconStyle::Nerd);

        let plain_text = buffer_text(&render(&plain, 100, 20));
        let nerd_text = buffer_text(&render(&nerd, 100, 20));

        assert!(!plain_text.contains('\u{f07b}'), "no glyph by default");
        assert!(nerd_text.contains('\u{f07b}'), "folder glyph missing");
        assert!(nerd_text.contains('\u{e7a8}'), "rust glyph missing");
        assert!(nerd_text.contains('\u{e62c}'), "elm glyph missing");
        assert!(nerd_text.contains("main.rs"), "the label survives");
    }

    #[test]
    fn a_wide_terminal_shows_both_diff_sides_with_revision_labels() {
        let app = diff_app(vec![file_diff(
            "a.rs",
            Some("old line\n"),
            Some("new line\n"),
        )]);
        let text = buffer_text(&render(&app, 120, 20));
        assert!(text.contains("HEAD~1"), "old label missing: {text}");
        assert!(text.contains("HEAD"), "new label missing: {text}");
        assert!(text.contains("old line"), "old side missing: {text}");
        assert!(text.contains("new line"), "new side missing: {text}");
    }

    #[test]
    fn a_narrow_diff_stacks_old_over_new_when_focused() {
        let mut app = diff_app(vec![file_diff(
            "a.rs",
            Some("old line\n"),
            Some("new line\n"),
        )]);
        focus_content(&mut app);
        let text = buffer_text(&render(&app, 60, 20));
        assert!(text.contains("old line"), "{text}");
        assert!(text.contains("new line"), "{text}");
    }

    #[test]
    fn a_narrow_terminal_shows_only_the_focused_pane() {
        let app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
        let tree = buffer_text(&render(&app, 60, 20));
        assert!(tree.contains("Files"));
        assert!(!tree.contains("pub fn a();"), "body must be hidden: {tree}");

        let mut body = app;
        focus_content(&mut body);
        let body = buffer_text(&render(&body, 60, 20));
        assert!(!body.contains("Files"), "tree must be hidden: {body}");
        assert!(body.contains("pub fn a();"));
    }

    #[test]
    fn a_tiny_terminal_shows_a_message_instead_of_a_frame() {
        let app = two_files();
        let text = buffer_text(&render(&app, 30, 5));
        assert!(text.contains("terminal too small"), "{text}");
    }

    #[test]
    fn an_empty_result_renders_a_clear_empty_state() {
        let app = show_app(vec![projected("empty.rs", "")]);
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("No files"), "{text}");
        assert!(
            text.contains("No projected file content in this scope."),
            "{text}"
        );
    }

    #[test]
    fn a_diff_with_no_rows_renders_a_clear_empty_state() {
        let app = diff_app(Vec::new());
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("No changes"), "{text}");
        assert!(text.contains("Body-only edits are omitted."), "{text}");
    }

    #[test]
    fn a_long_line_is_clipped_and_never_overwrites_the_tree() {
        let long = "x".repeat(300);
        let app = show_app(vec![projected("a.rs", &format!("{long}\n"))]);
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("Files"), "the tree must survive: {text}");
        assert!(!text.contains(&long), "the long line must be clipped");
        assert!(text.contains(&"x".repeat(40)), "the visible prefix remains");
    }

    #[test]
    fn responsive_boundaries_pick_the_right_layout() {
        let app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
        let wide = buffer_text(&render(&app, 80, 20));
        assert!(wide.contains("Files") && wide.contains("pub fn a();"));

        let narrow = buffer_text(&render(&app, 79, 20));
        assert!(narrow.contains("Files") && !narrow.contains("pub fn a();"));

        assert!(!buffer_text(&render(&app, 40, 8)).contains("terminal too small"));
        assert!(buffer_text(&render(&app, 39, 8)).contains("terminal too small"));
        assert!(buffer_text(&render(&app, 100, 7)).contains("terminal too small"));
    }

    #[test]
    fn horizontal_scroll_shifts_the_visible_columns() {
        let mut app = show_app(vec![projected("a.rs", "abcdef\n")]);
        focus_content(&mut app);
        assert!(buffer_text(&render(&app, 100, 20)).contains("abcdef"));

        let (scrolled, _) = update(Msg::Key(Key::Char('l')), &app);
        let text = buffer_text(&render(&scrolled, 100, 20));
        assert!(text.contains("bcdef"), "the tail must remain: {text}");
        assert!(
            !text.contains("abcdef"),
            "the first column must scroll away"
        );
    }

    #[test]
    fn the_show_pane_carries_syntax_colors() {
        if !crate::theme::colors_enabled() {
            return;
        }
        let app = show_app(vec![projected("a.rs", "pub struct User;\n")]);
        let buffer = render(&app, 100, 20);
        let colored = (0..buffer.area.height).any(|y| {
            (0..buffer.area.width).any(|x| {
                buffer
                    .cell((x, y))
                    .is_some_and(|cell| cell.fg != Color::Reset)
            })
        });
        assert!(colored, "expected a syntax foreground in the show pane");
    }

    #[test]
    fn non_utf8_path_components_render_escaped() {
        let path =
            SupportedPath::new(RepoPath::new(b"src/\xFF/lib.rs".as_slice()).expect("valid path"))
                .expect("supported path");
        let file = ProjectedFile::try_new(path, vec![item("x\n")]).expect("valid fixture");
        let app = show_app(vec![file]);
        let labels: Vec<String> = app.tree.rows.iter().map(|row| row.label.clone()).collect();
        assert!(labels.iter().any(|label| label == "src"), "{labels:?}");
        assert!(
            labels.iter().any(|label| label.contains("\\xFF")),
            "{labels:?}"
        );
    }

    #[test]
    fn a_long_diff_line_is_wrapped_inside_its_pane() {
        let long = "y".repeat(300);
        let app = diff_app(vec![file_diff(
            "a.rs",
            Some("x\n"),
            Some(&format!("{long}\n")),
        )]);
        let text = buffer_text(&render(&app, 120, 20));
        assert!(text.contains('…'), "expected continuation markers: {text}");
        assert!(!text.contains(&long), "the long line must be wrapped");
    }

    #[test]
    fn the_help_overlay_lists_the_keys() {
        let mut app = two_files();
        app.overlay = Some(Overlay::Help);
        let text = buffer_text(&render(&app, 100, 20));
        assert!(text.contains("help"), "{text}");
        assert!(text.contains("quit"), "{text}");
    }

    #[test]
    fn a_changed_diff_row_gets_delta_backgrounds() {
        if !crate::theme::colors_enabled() {
            return;
        }
        let app = diff_app(vec![file_diff(
            "a.rs",
            Some("pub id: u32;\n"),
            Some("pub id: u64;\n"),
        )]);
        let buffer = render(&app, 120, 20);
        let theme = &app.chrome.theme;
        let delete_bg = theme.color(theme.palette.del_bg);
        let add_bg = theme.color(theme.palette.add_bg);
        let add_emph = theme.color(theme.palette.add_emph);
        let del_emph = theme.color(theme.palette.del_emph);
        let mut delete = false;
        let mut add = false;
        let mut emphasis = false;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    delete |= cell.bg == delete_bg;
                    add |= cell.bg == add_bg;
                    emphasis |= cell.bg == add_emph || cell.bg == del_emph;
                }
            }
        }
        assert!(delete, "expected a removed-line background");
        assert!(add, "expected an added-line background");
        assert!(emphasis, "expected intra-line emphasis");
    }

    #[test]
    fn the_canvas_is_opaque_in_every_flavor() {
        use crate::theme::{Capability, Flavor};

        for flavor in [Flavor::Dark, Flavor::Light] {
            for overlay in [None, Some(Overlay::Help)] {
                let mut app = two_files();
                app.chrome.theme = Theme::new(flavor, Capability::TrueColor);
                app.overlay = overlay;
                let buffer = render(&app, 100, 20);
                for y in 0..buffer.area.height {
                    for x in 0..buffer.area.width {
                        let cell = buffer.cell((x, y)).expect("cell");
                        assert_ne!(
                            cell.bg,
                            Color::Reset,
                            "the terminal background leaked at ({x}, {y}) with {flavor:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_canvas_takes_the_light_palette_background() {
        use crate::theme::{Capability, Flavor};

        let mut app = two_files();
        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        let bg = light.color(light.palette.bg);
        app.chrome.theme = light;
        let buffer = render(&app, 100, 20);
        let mut count = 0;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer.cell((x, y)).is_some_and(|cell| cell.bg == bg) {
                    count += 1;
                }
            }
        }
        assert!(count > 0, "expected the light background somewhere");
    }

    #[test]
    fn clip_line_slices_by_display_width() {
        use crate::view::text::clip_line;
        assert_eq!(clip_line("abcdef", 2, 3), "cde");
        assert_eq!(clip_line("abcdef", 0, 0), "");
        assert_eq!(clip_line("abcdef", 10, 3), "");
        assert_eq!(clip_line("a\tb", 0, 10), "a    b");
        assert_eq!(clip_line("日本", 0, 3), "日");
        assert_eq!(clip_line("日本", 1, 4), "本");
    }

    #[test]
    fn window_offset_keeps_the_cursor_visible() {
        use crate::view::geom::window_offset;
        assert_eq!(window_offset(0, 100, 10), 0);
        assert_eq!(window_offset(9, 100, 10), 0);
        assert_eq!(window_offset(10, 100, 10), 1);
        assert_eq!(window_offset(0, 3, 10), 0);
        assert_eq!(window_offset(5, 100, 0), 0);
    }
}
