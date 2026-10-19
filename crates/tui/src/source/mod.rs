//! The source view plugin: one revision's source explorer.
//!
//! It owns the loaded revision, the outline set, the body cursor and scrolls,
//! the fold overrides, and the derived visible rows. It never owns the selected
//! path, the search, the mode, or the scope: those live on the kernel's
//! [`crate::kernel::workspace::Workspace`], and the view reads them through
//! [`crate::kernel::cx::Cx`].
//!
//! The body cursor always names a visible row's source line, so the `tests`-mode
//! filter (which can hide the cursor's line) anchors it to the nearest visible
//! row; both body offsets are clamped to the projection before the cursor is
//! revealed.
//!
//! In `tests` mode a dropped region's whole subtree is skipped. That is safe
//! because a container that holds a test is itself retained, so a dropped
//! container can have no retained descendant.

mod folds;
mod rows;
mod update;
mod view;

use std::sync::Arc;

use base::{ProjectionMode, SourceOutline};
use engine::{Engine, EngineError};
use ratatui::Frame;

use crate::kernel::chrome::{Chip, ChipTone, Segment, SegmentTone};
use crate::kernel::command::{Binding, CommandId, CommandSpec, Group, KeyScope, KeySeq, Owner};
use crate::kernel::cx::Cx;
use crate::kernel::help::{HelpEntry, HelpSection, MOUSE, VIEW};
use crate::kernel::layout::{PaneGeometry, PaneSpec, Placement};
use crate::kernel::plugin::{
    Event, FocusKey, HintTarget, Input, Out, PaneId, Plugin, PluginId, Reply, Request,
};
use crate::plugin::{SOURCE, SOURCE_FOCUS, SOURCE_PANE};
use crate::util::input::Key;

use folds::{Fold, fold_command};
pub(crate) use folds::{FoldState, ShowFolds, ShowHighlights};
#[allow(unused_imports)]
pub(crate) use rows::VisibleRow;
pub(crate) use rows::{ShowBody, VisibleRows};
pub(crate) use update::{Cmd, Msg};

/// A request to project one revision's source outlines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShowRequest {
    pub revision: String,
    pub mode: ProjectionMode,
    pub selection: base::Selection,
}

const MOUSE_SCROLL_STEP: i32 = 3;

/// The tag the revision prompt echoes back with its value.
pub(crate) const REVISION_PROMPT_TAG: u8 = 0;

const FOLD: Group = Group {
    prefix: 'z',
    name: "fold",
};

const FOLD_SECTION: HelpSection = HelpSection {
    title: "Fold",
    rank: 20,
};

/// The source view's state.
#[derive(Clone, Debug)]
pub struct SourceView {
    pub revision: String,
    pub loaded_mode: ProjectionMode,
    pub files: Arc<[SourceOutline]>,
    pub body: ShowBody,
    pub highlight: ShowHighlights,
    pub folds: ShowFolds,
    pub fold_state: FoldState,
    pub(crate) visible_rows: VisibleRows,
    pub loading: bool,
    active: bool,
}

impl SourceView {
    /// Build the view for a `show` session. The startup load fills it in.
    pub(crate) fn new(revision: String, mode: ProjectionMode) -> Self {
        Self {
            revision,
            loaded_mode: mode,
            files: Arc::from(Vec::new()),
            body: ShowBody {
                scroll: 0,
                hscroll: 0,
                cursor_line: 1,
            },
            highlight: ShowHighlights::default(),
            folds: ShowFolds::default(),
            fold_state: FoldState::default(),
            visible_rows: VisibleRows::default(),
            loading: true,
            active: true,
        }
    }

    /// Build an inactive view for a session of another kind.
    pub(crate) fn inactive() -> Self {
        Self {
            revision: String::new(),
            loaded_mode: ProjectionMode::Types,
            files: Arc::from(Vec::new()),
            body: ShowBody {
                scroll: 0,
                hscroll: 0,
                cursor_line: 1,
            },
            highlight: ShowHighlights::default(),
            folds: ShowFolds::default(),
            fold_state: FoldState::default(),
            visible_rows: VisibleRows::default(),
            loading: false,
            active: false,
        }
    }

    /// The load request for `selection`, projected with the currently loaded
    /// mode.
    pub(crate) fn request(&self, selection: base::Selection) -> ShowRequest {
        ShowRequest {
            revision: self.revision.clone(),
            mode: self.loaded_mode,
            selection,
        }
    }

    fn selected<'w>(&self, cx: &'w Cx) -> Option<&'w base::RepoPath> {
        cx.workspace.selected.as_ref()
    }
}

/// The source view's commands. They exist only while the view is active.
fn commands() -> Vec<CommandSpec> {
    let mut specs = vec![CommandSpec {
        id: CommandId {
            owner: Owner::Show,
            name: "revision.edit",
        },
        label: "Edit revision",
        hint: "r",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::Char('r')),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank: 200,
    }];
    for (name, label, hint, second, rank) in [
        ("fold.toggle", "Toggle fold", "za", 'a', 210),
        ("fold.open", "Open fold", "zo", 'o', 211),
        ("fold.close", "Close fold", "zc", 'c', 212),
        (
            "fold.open_recursive",
            "Open fold recursively",
            "zO",
            'O',
            213,
        ),
        (
            "fold.close_recursive",
            "Close fold recursively",
            "zC",
            'C',
            214,
        ),
        ("fold.open_all", "Open all folds", "zR", 'R', 215),
        ("fold.close_all", "Close all folds", "zM", 'M', 216),
    ] {
        specs.push(CommandSpec {
            id: CommandId {
                owner: Owner::Show,
                name,
            },
            label,
            hint,
            bindings: vec![Binding {
                seq: KeySeq::Prefixed('z', second),
                scope: KeyScope::Focused(SOURCE_FOCUS),
            }],
            palette: true,
            group: Some(FOLD),
            rank,
        });
    }
    specs
}

fn help() -> Vec<HelpEntry> {
    vec![
        HelpEntry {
            section: FOLD_SECTION,
            keys: "Enter / za",
            text: "toggle the fold at the cursor",
            rank: 10,
        },
        HelpEntry {
            section: FOLD_SECTION,
            keys: "zo / zc",
            text: "open or close the fold",
            rank: 20,
        },
        HelpEntry {
            section: FOLD_SECTION,
            keys: "zO / zC",
            text: "open or close recursively",
            rank: 30,
        },
        HelpEntry {
            section: FOLD_SECTION,
            keys: "zR / zM",
            text: "unfold all / fold all",
            rank: 40,
        },
        HelpEntry {
            section: VIEW,
            keys: "r",
            text: "edit the revision",
            rank: 40,
        },
        HelpEntry {
            section: MOUSE,
            keys: "gutter + / -",
            text: "fold or unfold a declaration",
            rank: 20,
        },
        HelpEntry {
            section: MOUSE,
            keys: "click line",
            text: "move the source cursor",
            rank: 30,
        },
    ]
}

impl Plugin for SourceView {
    const ID: PluginId = SOURCE;
    type Msg = Msg;
    type Cmd = Cmd;

    fn panes(&self, cx: &Cx) -> Vec<PaneSpec> {
        if !cx.is_active_view(SOURCE) {
            return Vec::new();
        }
        vec![PaneSpec {
            id: SOURCE_PANE,
            placement: Placement::Body,
            focus: SOURCE_FOCUS,
        }]
    }

    fn has_focus(&self, focus: FocusKey) -> bool {
        self.active && focus == SOURCE_FOCUS
    }

    fn first_focus(&self) -> Option<FocusKey> {
        self.active.then_some(SOURCE_FOCUS)
    }

    fn commands(&self, cx: &Cx) -> Vec<CommandSpec> {
        if cx.is_active_view(SOURCE) {
            commands()
        } else {
            Vec::new()
        }
    }

    fn help(&self, cx: &Cx) -> Vec<HelpEntry> {
        if cx.is_active_view(SOURCE) {
            help()
        } else {
            Vec::new()
        }
    }

    fn hints(&self, target: HintTarget, cx: &Cx) -> Option<&'static str> {
        match target {
            HintTarget::Focus(focus) if focus == SOURCE_FOCUS && cx.is_active_view(SOURCE) => {
                Some("wheel / j k scroll   / search   Ctrl-P commands   Tab tree   ? help ")
            }
            HintTarget::Focus(_) | HintTarget::Overlay(_) => None,
        }
    }

    fn chips(&self, cx: &Cx) -> Vec<Chip> {
        if !cx.is_active_view(SOURCE) {
            return Vec::new();
        }
        vec![Chip {
            label: self.revision.clone(),
            tone: ChipTone::Neutral,
        }]
    }

    fn status(&self, cx: &Cx) -> Vec<Segment> {
        if !cx.is_active_view(SOURCE) {
            return Vec::new();
        }
        let selected = cx.workspace.selected.as_ref();
        if self.active_source(selected).is_none() {
            return Vec::new();
        }
        vec![Segment {
            text: format!(
                "{} of {} lines ",
                self.line_count(),
                self.source_line_count(selected)
            ),
            tone: SegmentTone::Muted,
        }]
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
        if pane == SOURCE_PANE && cx.is_active_view(SOURCE) {
            view::render(self, cx, frame, geometry.outer, focused, geometry.edge);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Cmd> {
        if pane != SOURCE_PANE || !cx.is_active_view(SOURCE) {
            return Reply::unhandled();
        }
        match input {
            Input::Key(key) => {
                self.ensure_visible_rows(self.selected(cx));
                match key {
                    Key::Enter => {
                        self.fold_action(Fold::Toggle, cx);
                        Reply::handled(Vec::new())
                    }
                    Key::Down
                    | Key::Char('j')
                    | Key::Up
                    | Key::Char('k')
                    | Key::Right
                    | Key::Char('l')
                    | Key::Left
                    | Key::Char('h') => {
                        update::body_key(self, key, cx);
                        Reply::handled(Vec::new())
                    }
                    _ => Reply::unhandled(),
                }
            }
            Input::Nav(nav) => {
                self.ensure_visible_rows(self.selected(cx));
                update::cursor_action(self, nav, cx);
                Reply::handled(Vec::new())
            }
            Input::Mouse(event) => {
                update::mouse(self, event, cx);
                Reply::handled(Vec::new())
            }
        }
    }

    fn run(&mut self, command: CommandId, cx: &Cx) -> Vec<Out<Cmd>> {
        if command.owner != Owner::Show || !cx.is_active_view(SOURCE) {
            return Vec::new();
        }
        if command.name == "revision.edit" {
            return vec![Out::Kernel(Request::OpenPrompt {
                label: "revision",
                initial: self.revision.clone(),
                tag: REVISION_PROMPT_TAG,
            })];
        }
        if let Some(action) = fold_command(command.name) {
            self.fold_action(action, cx);
        }
        Vec::new()
    }

    fn update(&mut self, msg: Msg, _cx: &Cx) -> Vec<Out<Cmd>> {
        match msg {
            Msg::ShowLoaded { request, result } => update::loaded(self, request, result),
        }
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> Vec<Out<Cmd>> {
        if !cx.is_active_view(SOURCE) {
            return Vec::new();
        }
        update::on_event(self, event, cx)
    }

    fn search(&mut self, needle: &str, cx: &Cx) -> Vec<crate::kernel::search::SearchMatch> {
        if !cx.is_active_view(SOURCE) {
            return Vec::new();
        }
        self.ensure_visible_rows(self.selected(cx));
        self.search_matches(needle, cx.workspace)
    }

    fn settle(&mut self, cx: &Cx) {
        if !cx.is_active_view(SOURCE) {
            return;
        }
        let selected = cx.workspace.selected.clone();
        self.ensure_visible_rows(selected.as_ref());
        if cx.body_visible() {
            self.ensure_highlight(cx.theme, selected.as_ref());
        }
    }

    fn perform(engine: &Engine, cmd: Cmd) -> Msg {
        match cmd {
            Cmd::Load(request) => {
                let result = engine
                    .show_source_outlines(&request.revision, request.mode, &request.selection)
                    .map(Arc::from)
                    .map_err(Box::new);
                Msg::ShowLoaded { request, result }
            }
        }
    }

    fn startup_failure(msg: Msg) -> Result<Msg, Box<EngineError>> {
        match msg {
            Msg::ShowLoaded {
                result: Err(error), ..
            } => Err(error),
            other => Ok(other),
        }
    }
}
