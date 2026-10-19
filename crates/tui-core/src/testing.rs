//! Test doubles: fake plugins, a `Cx` builder, and small kernel fixtures.
//!
//! Compiled only for this crate's tests or with the `testing` feature, which
//! `tui` and `tui-diff` enable in their dev-dependencies. Nothing here is part
//! of the kernel's contract: the fakes deliberately know nothing about a real
//! plugin, so a kernel test that passes here proves the kernel works without
//! one.

use base::{ProjectionMode, RepoPath, Selection};
use engine::{Engine, EngineError};

use crate::app::{App, Chrome, Kernel};
use crate::command::{Binding, CommandId, CommandSpec, KERNEL, KeyScope, KeySeq};
use crate::cx::Cx;
use crate::help::HelpEntry;
use crate::keymap::Keymap;
use crate::layout::{PaneSpec, Placement};
use crate::plugin::{
    Event, FocusKey, Input, Out, PaneId, Plugin, PluginId, Reply, Request, Size, ViewId,
};
use crate::plugin_set;
use crate::render::icons::{IconStyle, Icons};
use crate::render::theme::Theme;
use crate::search::{SearchMatch, SearchPane};
use crate::util::input::Key;
use crate::workspace::{FileSet, Query, Scope, Workspace};

/// The fake navigator plugin's id.
pub const NAV: PluginId = PluginId("fake-nav");

/// The fake body-view plugin's id.
pub const VIEW: PluginId = PluginId("fake-view");

/// The fake overlay plugin's id.
pub const OVERLAY: PluginId = PluginId("fake-overlay");

/// The fake navigator's pane.
pub const NAV_PANE: PaneId = PaneId {
    owner: NAV,
    slot: 0,
};

/// The fake navigator's focus key.
pub const NAV_FOCUS: FocusKey = FocusKey {
    owner: NAV,
    slot: 0,
};

/// The fake view's body pane.
pub const BODY_PANE: PaneId = PaneId {
    owner: VIEW,
    slot: 0,
};

/// The fake view's body focus key.
pub const BODY_FOCUS: FocusKey = FocusKey {
    owner: VIEW,
    slot: 0,
};

/// The fake overlay's pane.
pub const OVERLAY_PANE: PaneId = PaneId {
    owner: OVERLAY,
    slot: 0,
};

/// A second fake overlay pane that is not modal, so keys fall through it.
pub const PASS_PANE: PaneId = PaneId {
    owner: OVERLAY,
    slot: 1,
};

/// The fake view's active view id.
pub const ACTIVE_VIEW: ViewId = ViewId {
    owner: VIEW,
    slot: 0,
};

/// The fake view's commands as `(name, key)`; the key binds the command.
pub const VIEW_COMMAND: CommandId = CommandId {
    owner: VIEW,
    name: "view.bump",
};

/// The fake navigator's resize command.
pub const NAV_COMMAND: CommandId = CommandId {
    owner: NAV,
    name: "nav.wider",
};

const NAV_SPEC: PaneSpec = PaneSpec {
    id: NAV_PANE,
    placement: Placement::Navigator,
    focus: NAV_FOCUS,
};

const BODY_SPEC: PaneSpec = PaneSpec {
    id: BODY_PANE,
    placement: Placement::Body,
    focus: BODY_FOCUS,
};

/// A fake plugin effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FakeCmd {
    /// Produces [`FakeMsg::Done`].
    Bump,
    /// Produces [`FakeMsg::Failed`].
    Fail,
}

/// A fake plugin message.
#[derive(Debug)]
pub enum FakeMsg {
    Done,
    Failed(Box<EngineError>),
}

/// What a fake plugin saw, for the kernel tests to assert on.
#[derive(Clone, Default, Debug)]
pub struct Log {
    pub events: Vec<&'static str>,
    pub keys: Vec<Key>,
    pub settles: usize,
    pub updates: usize,
    pub searches: usize,
}

/// The fake navigator: a tree-like pane with one resize command.
#[derive(Clone, Default)]
pub struct Nav {
    pub log: Log,
}

/// The fake body view: one pane, one command, and two search matches.
#[derive(Clone, Default)]
pub struct View {
    pub log: Log,
}

/// The fake overlay: an overlay pane and no layout surface.
#[derive(Clone, Default)]
pub struct Overlay {
    pub log: Log,
}

plugin_set! {
    /// The fake plugin set the kernel tests drive.
    pub struct Fakes: FakesMsg, FakesCmd {
        nav: Nav = Nav,
        view: View = View,
        overlay: Overlay = Overlay,
    }
}

fn record(log: &mut Log, event: &Event) {
    log.events.push(event_name(event));
}

/// The stable name of an event, for log assertions.
pub fn event_name(event: &Event) -> &'static str {
    match event {
        Event::FilesChanged => "FilesChanged",
        Event::SelectionChanged => "SelectionChanged",
        Event::QueryChanged => "QueryChanged",
        Event::SearchChanged => "SearchChanged",
        Event::PrefixPending(_) => "PrefixPending",
        Event::PrefixCleared => "PrefixCleared",
        Event::Resized => "Resized",
        Event::OverlayClosed(_) => "OverlayClosed",
        Event::RevealMatch(_) => "RevealMatch",
        Event::Requery(_) => "Requery",
        Event::PromptRequested { .. } => "PromptRequested",
        Event::PromptSubmitted { .. } => "PromptSubmitted",
    }
}

impl Plugin for Nav {
    const ID: PluginId = NAV;
    type Msg = FakeMsg;
    type Cmd = FakeCmd;

    fn panes(&self, _cx: &Cx) -> Vec<PaneSpec> {
        vec![NAV_SPEC]
    }

    fn has_focus(&self, focus: FocusKey) -> bool {
        focus == NAV_FOCUS
    }

    fn first_focus(&self) -> Option<FocusKey> {
        Some(NAV_FOCUS)
    }

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        vec![CommandSpec {
            id: NAV_COMMAND,
            label: "Widen",
            hint: "]",
            bindings: vec![Binding {
                seq: KeySeq::Key(Key::Char(']')),
                scope: KeyScope::Anywhere,
            }],
            palette: true,
            group: None,
            rank: 330,
        }]
    }

    fn run(&mut self, command: CommandId, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        match command.name {
            "nav.wider" => vec![Out::Kernel(Request::ResizeNavigator(
                crate::plugin::NavigatorResize::Wider,
            ))],
            _ => Vec::new(),
        }
    }

    fn update(&mut self, _msg: FakeMsg, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        self.log.updates += 1;
        Vec::new()
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        record(&mut self.log, event);
        Vec::new()
    }

    fn settle(&mut self, _cx: &Cx) {
        self.log.settles += 1;
    }

    fn perform(_engine: &Engine, cmd: FakeCmd) -> FakeMsg {
        match cmd {
            FakeCmd::Bump => FakeMsg::Done,
            FakeCmd::Fail => FakeMsg::Failed(Box::new(load_failure(_engine))),
        }
    }

    fn startup_failure(msg: FakeMsg) -> Result<FakeMsg, Box<EngineError>> {
        match msg {
            FakeMsg::Failed(error) => Err(error),
            other => Ok(other),
        }
    }
}

impl Plugin for View {
    const ID: PluginId = VIEW;
    type Msg = FakeMsg;
    type Cmd = FakeCmd;

    fn panes(&self, cx: &Cx) -> Vec<PaneSpec> {
        if cx.is_active_view(VIEW) {
            vec![BODY_SPEC]
        } else {
            Vec::new()
        }
    }

    fn has_focus(&self, focus: FocusKey) -> bool {
        focus == BODY_FOCUS
    }

    fn first_focus(&self) -> Option<FocusKey> {
        Some(BODY_FOCUS)
    }

    fn commands(&self, cx: &Cx) -> Vec<CommandSpec> {
        if !cx.is_active_view(VIEW) {
            return Vec::new();
        }
        vec![
            CommandSpec {
                id: VIEW_COMMAND,
                label: "Bump",
                hint: "x",
                bindings: vec![Binding {
                    seq: KeySeq::Key(Key::Char('x')),
                    scope: KeyScope::Anywhere,
                }],
                palette: true,
                group: None,
                rank: 200,
            },
            CommandSpec {
                id: CommandId {
                    owner: VIEW,
                    name: "view.fold",
                },
                label: "Toggle fold",
                hint: "za",
                bindings: vec![Binding {
                    seq: KeySeq::Prefixed('z', 'a'),
                    scope: KeyScope::Focused(BODY_FOCUS),
                }],
                palette: true,
                group: Some(crate::command::Group {
                    prefix: 'z',
                    name: "fold",
                }),
                rank: 210,
            },
        ]
    }

    fn run(&mut self, command: CommandId, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        match command.name {
            "view.bump" => vec![Out::Effect(FakeCmd::Bump)],
            _ => Vec::new(),
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<FakeCmd> {
        if pane.owner != VIEW {
            return Reply::unhandled();
        }
        if let Input::Key(key) = input {
            self.log.keys.push(key);
        }
        Reply::unhandled()
    }

    fn update(&mut self, _msg: FakeMsg, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        self.log.updates += 1;
        Vec::new()
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        record(&mut self.log, event);
        match event {
            Event::Requery(_) => vec![Out::Effect(FakeCmd::Bump)],
            _ => Vec::new(),
        }
    }

    fn search(&mut self, needle: &str, _cx: &Cx) -> Vec<SearchMatch> {
        self.log.searches += 1;
        if needle == "alpha" {
            vec![
                SearchMatch {
                    pane: SearchPane::Source,
                    line: 1,
                    start: 0,
                    end: 5,
                },
                SearchMatch {
                    pane: SearchPane::Source,
                    line: 2,
                    start: 0,
                    end: 5,
                },
            ]
        } else {
            Vec::new()
        }
    }

    fn settle(&mut self, _cx: &Cx) {
        self.log.settles += 1;
    }

    fn perform(_engine: &Engine, cmd: FakeCmd) -> FakeMsg {
        Nav::perform(_engine, cmd)
    }

    fn startup_failure(msg: FakeMsg) -> Result<FakeMsg, Box<EngineError>> {
        Nav::startup_failure(msg)
    }
}

impl Plugin for Overlay {
    const ID: PluginId = OVERLAY;
    type Msg = FakeMsg;
    type Cmd = FakeCmd;

    fn overlay_spec(&self, pane: PaneId) -> crate::plugin::OverlaySpec {
        if pane == PASS_PANE {
            crate::plugin::OverlaySpec {
                captures_text: true,
                modal: false,
                keeps_ctrl_jk: true,
            }
        } else {
            crate::plugin::OverlaySpec {
                captures_text: true,
                modal: true,
                keeps_ctrl_jk: false,
            }
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<FakeCmd> {
        if pane.owner != OVERLAY {
            return Reply::unhandled();
        }
        if let Input::Key(key) = input {
            self.log.keys.push(key);
        }
        Reply::unhandled()
    }

    fn update(&mut self, _msg: FakeMsg, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        self.log.updates += 1;
        Vec::new()
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<FakeCmd>> {
        record(&mut self.log, event);
        Vec::new()
    }

    fn settle(&mut self, _cx: &Cx) {
        self.log.settles += 1;
    }

    fn perform(_engine: &Engine, cmd: FakeCmd) -> FakeMsg {
        Nav::perform(_engine, cmd)
    }

    fn startup_failure(msg: FakeMsg) -> Result<FakeMsg, Box<EngineError>> {
        Nav::startup_failure(msg)
    }
}

fn load_failure(engine: &Engine) -> EngineError {
    engine
        .show_source_outlines("no-such-revision", ProjectionMode::Types, &Selection::all())
        .expect_err("the fake revision cannot resolve")
}

/// The fake kernel commands, so a test can run `RunCommand` on the kernel too.
pub fn kernel_command(name: &'static str) -> CommandId {
    CommandId {
        owner: KERNEL,
        name,
    }
}

/// An empty workspace over `all`.
pub fn workspace() -> Workspace {
    Workspace {
        query: Query {
            mode: ProjectionMode::Types,
            scope: Scope::new(Selection::all()),
        },
        files: FileSet::default(),
        selected: None,
        search: None,
    }
}

/// A kernel with the fake panes and `active_view`.
pub fn kernel(active_view: ViewId) -> Kernel {
    Kernel {
        root: "/repo".to_owned(),
        size: Size {
            width: 100,
            height: 30,
        },
        chrome: Chrome {
            tree_percent: 30,
            theme: Theme::dark(),
            icons: Icons::new(IconStyle::None),
            spinner: 0,
            quit: false,
        },
        workspace: workspace(),
        focus: Some(NAV_FOCUS),
        active_view,
        overlay: None,
        keymap: Keymap::default(),
        diagnostic: None,
    }
}

/// An app with the fake plugin set and the fake view active.
pub fn app() -> App<Fakes> {
    App::new(
        kernel(ACTIVE_VIEW),
        Fakes {
            nav: Nav::default(),
            view: View::default(),
            overlay: Overlay::default(),
        },
    )
}

/// A `Cx` over `panes` with no collected commands or help.
#[allow(clippy::too_many_arguments)]
pub fn cx<'a>(
    size: Size,
    theme: &'a Theme,
    icons: &'a Icons,
    workspace: &'a Workspace,
    focus: Option<FocusKey>,
    active_view: ViewId,
    panes: &'a [PaneSpec],
) -> Cx<'a> {
    cx_with(
        size,
        theme,
        icons,
        workspace,
        focus,
        active_view,
        panes,
        &[],
        &[],
    )
}

/// A `Cx` with the collected declarations, for tests that read them.
#[allow(clippy::too_many_arguments)]
pub fn cx_with<'a>(
    size: Size,
    theme: &'a Theme,
    icons: &'a Icons,
    workspace: &'a Workspace,
    focus: Option<FocusKey>,
    active_view: ViewId,
    panes: &'a [PaneSpec],
    commands: &'a [CommandSpec],
    help: &'a [HelpEntry],
) -> Cx<'a> {
    Cx::new(
        size,
        theme,
        icons,
        "/repo",
        workspace,
        focus,
        active_view,
        None,
        None,
        false,
        30,
        false,
        panes,
        commands,
        help,
    )
}

/// A selected repo path.
pub fn path(path: &str) -> RepoPath {
    RepoPath::new(path).expect("valid test path")
}
