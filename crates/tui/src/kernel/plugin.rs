//! The plugin contract: identifiers, the `Plugin` and `PluginSet` traits, and
//! the request/event vocabulary the kernel and plugins exchange.
//!
//! This is the kernel's public surface to a plugin. A plugin never names another
//! plugin: it reads shared state through [`Cx`] and asks for changes through
//! [`Out::Kernel`] requests. The kernel never names a plugin.
//!
//! The module is `pub(crate)` in this phase; the crate split in Phase 5 makes
//! it public.

#![allow(dead_code)]
use std::fmt;

use engine::{Engine, EngineError};
use ratatui::Frame;
use ratatui::layout::Rect;

use base::{ProjectionMode, RepoPath, Selection};

use crate::kernel::chrome::{Chip, Segment};
use crate::kernel::command::CommandSpec;
use crate::kernel::cx::Cx;
use crate::kernel::help::HelpEntry;
use crate::kernel::layout::{PaneGeometry, PaneSpec};
use crate::kernel::nav::Nav;
use crate::kernel::search::SearchMatch;
use crate::util::input::{Key, MouseKind};

/// A plugin's stable identity. `PluginId("kernel")` is reserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PluginId(pub &'static str);

/// The terminal size in cells.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

/// One pane a plugin owns. `slot` distinguishes the panes of one owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PaneId {
    pub owner: PluginId,
    pub slot: u8,
}

/// The unit focus moves between. Panes that scroll together share one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FocusKey {
    pub owner: PluginId,
    pub slot: u8,
}

/// One addressable view a plugin can activate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ViewId {
    pub owner: PluginId,
    pub slot: u8,
}

/// An uninhabited message or effect type for plugins that have none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Never {}

/// What a plugin wants the kernel to do with the outside world.
pub enum Out<C> {
    /// Run an engine effect through the runtime.
    Effect(C),
    /// Ask the kernel for a shared-state change.
    Kernel(Request),
}

/// A plugin's answer to an input: whether it was handled, and what to do next.
pub struct Reply<C> {
    pub handled: bool,
    pub outs: Vec<Out<C>>,
}

impl<C> Reply<C> {
    pub fn unhandled() -> Self {
        Self {
            handled: false,
            outs: Vec::new(),
        }
    }

    pub fn handled(outs: Vec<Out<C>>) -> Self {
        Self {
            handled: true,
            outs,
        }
    }
}

/// Which pane's hints a plugin should contribute.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HintTarget {
    Focus(FocusKey),
    Overlay(PaneId),
}
/// How the kernel should treat keys for an open overlay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlaySpec {
    /// Printable keys go to the overlay, so `q` does not quit.
    pub captures_text: bool,
    /// Keys the overlay does not handle are swallowed instead of falling through.
    pub modal: bool,
    /// Ctrl-J / Ctrl-K reach the overlay as themselves instead of `j` / `k`.
    pub keeps_ctrl_jk: bool,
}

impl OverlaySpec {
    pub const MODAL: Self = Self {
        captures_text: false,
        modal: true,
        keeps_ctrl_jk: false,
    };
}

/// A kernel request. Depth-first application is in [`crate::kernel::dispatch`].
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Request {
    Quit,
    Diagnose(String),
    ClearDiagnostic,

    SetFiles(crate::kernel::workspace::FileSet),
    Select(Option<RepoPath>),
    Requery(QueryChange),
    QueryLoaded(crate::kernel::workspace::Query),

    SetSearch(String),
    CommitSearch,
    ClearSearch,

    Focus(FocusKey),
    FocusDefault,
    CycleFocus {
        forward: bool,
    },
    ResizeNavigator(NavigatorResize),

    OpenOverlay(PaneId),
    CloseOverlay(PaneId),

    OpenPrompt {
        label: &'static str,
        initial: String,
        tag: u8,
    },
    SubmitPrompt {
        to: PluginId,
        tag: u8,
        value: String,
    },

    CancelPrefix,
    RunCommand(crate::kernel::command::CommandId),
}

/// A navigator width change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigatorResize {
    Wider,
    Narrower,
    Reset,
}

/// A query change requested by the mode picker or the scope chooser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueryChange {
    Mode(ProjectionMode),
    Scope(Selection),
}

/// An event the kernel broadcasts or routes after a state change.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Event {
    FilesChanged,
    SelectionChanged,
    QueryChanged,
    SearchChanged,
    PrefixPending(char),
    PrefixCleared,
    Resized,
    OverlayClosed(PaneId),
    RevealMatch(SearchMatch),
    Requery(QueryChange),
    PromptRequested {
        from: PluginId,
        label: &'static str,
        initial: String,
        tag: u8,
    },
    PromptSubmitted {
        tag: u8,
        value: String,
    },
}

/// Who an event goes to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventTarget {
    All,
    One(PluginId),
}

/// Input a plugin receives for one pane.
#[derive(Clone, Copy, Debug)]
pub enum Input {
    Key(Key),
    Nav(Nav),
    Mouse(MouseInput),
}

/// A mouse event already hit-tested to a pane.
#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
    pub kind: MouseKind,
    pub column: u16,
    pub row: u16,
    pub geometry: PaneGeometry,
}

/// The plugin contract. Every method is pure or mutates only `self`.
pub trait Plugin: Clone + 'static {
    const ID: PluginId;
    type Msg: fmt::Debug;
    type Cmd: Clone + fmt::Debug + PartialEq;

    // Declarations: pure reads.
    fn panes(&self, cx: &Cx) -> Vec<PaneSpec> {
        let _ = cx;
        Vec::new()
    }
    /// Whether `focus` names one of this plugin's visible focus stops. The
    /// kernel calls this on every update step to normalise focus, so an
    /// implementation must not allocate.
    fn has_focus(&self, focus: FocusKey) -> bool {
        let _ = focus;
        false
    }
    /// The focus stop normalisation falls back to when the current one is
    /// gone: this plugin's first focus stop, or `None` when it has none.
    fn first_focus(&self) -> Option<FocusKey> {
        None
    }
    fn commands(&self, cx: &Cx) -> Vec<CommandSpec> {
        let _ = cx;
        Vec::new()
    }
    fn help(&self, cx: &Cx) -> Vec<HelpEntry> {
        let _ = cx;
        Vec::new()
    }
    fn hints(&self, target: HintTarget, cx: &Cx) -> Option<&'static str> {
        let _ = (target, cx);
        None
    }
    fn chips(&self, cx: &Cx) -> Vec<Chip> {
        let _ = cx;
        Vec::new()
    }
    fn status(&self, cx: &Cx) -> Vec<Segment> {
        let _ = cx;
        Vec::new()
    }
    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        let _ = pane;
        OverlaySpec::MODAL
    }
    /// Whether this plugin owns `pane` for `overlay_spec` routing. The default
    /// matches the pane's owner against this plugin's id; `Legacy` overrides it
    /// to claim the overlay owners whose plugins are not extracted yet.
    fn owns_pane(&self, pane: PaneId) -> bool {
        pane.owner == Self::ID
    }
    /// Whether this plugin answers `hints` for `focus`. The default matches the
    /// focus's owner; `Legacy` overrides it to claim the view focus keys whose
    /// plugins are not extracted yet.
    fn owns_focus(&self, focus: FocusKey) -> bool {
        focus.owner == Self::ID
    }
    fn busy(&self) -> bool {
        false
    }

    // Drawing: pure.
    fn render(
        &self,
        pane: PaneId,
        geometry: PaneGeometry,
        focused: bool,
        cx: &Cx,
        frame: &mut Frame,
    ) {
        let _ = (pane, geometry, focused, cx, frame);
    }
    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        let _ = (pane, area, cx, frame);
    }

    // Reactions: mutate only `self`.
    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Self::Cmd> {
        let _ = (pane, input, cx);
        Reply::unhandled()
    }
    fn run(&mut self, command: crate::kernel::command::CommandId, cx: &Cx) -> Vec<Out<Self::Cmd>> {
        let _ = (command, cx);
        Vec::new()
    }
    fn update(&mut self, msg: Self::Msg, cx: &Cx) -> Vec<Out<Self::Cmd>> {
        let _ = (msg, cx);
        Vec::new()
    }
    fn on_event(&mut self, event: &Event, cx: &Cx) -> Vec<Out<Self::Cmd>> {
        let _ = (event, cx);
        Vec::new()
    }
    fn search(&mut self, needle: &str, cx: &Cx) -> Vec<SearchMatch> {
        let _ = (needle, cx);
        Vec::new()
    }
    fn after_batch(&mut self, pending: &[Self::Cmd], cx: &Cx) -> Vec<Out<Self::Cmd>> {
        let _ = (pending, cx);
        Vec::new()
    }
    fn settle(&mut self, cx: &Cx) {
        let _ = cx;
    }

    // Effects: only the runtime calls these.
    fn perform(engine: &Engine, cmd: Self::Cmd) -> Self::Msg;
    fn startup_failure(msg: Self::Msg) -> Result<Self::Msg, Box<EngineError>> {
        Ok(msg)
    }
}

/// The compile-time set of installed plugins.
pub trait PluginSet: Clone + 'static {
    type Msg: fmt::Debug;
    type Cmd: Clone + fmt::Debug + PartialEq;

    /// Registration order.
    fn ids(&self) -> &'static [PluginId];
    fn panes(&self, cx: &Cx) -> Vec<PaneSpec>;
    /// Allocation-free focus membership, for per-step normalisation.
    fn has_focus(&self, focus: FocusKey) -> bool;
    /// The fallback focus stop of the first plugin that has one.
    fn first_focus(&self) -> Option<FocusKey>;
    fn commands(&self, cx: &Cx) -> Vec<CommandSpec>;
    fn help(&self, cx: &Cx) -> Vec<HelpEntry>;
    fn hints(&self, target: HintTarget, cx: &Cx) -> Option<&'static str>;
    fn chips(&self, cx: &Cx) -> Vec<Chip>;
    fn status(&self, cx: &Cx) -> Vec<Segment>;
    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec;
    /// The plugin that answers for `pane`: the first whose `owns_pane` claims
    /// it. `Legacy` claims the panes whose plugins are not extracted yet.
    fn pane_owner(&self, pane: PaneId) -> Option<PluginId>;
    fn busy(&self) -> bool;
    fn render(
        &self,
        pane: PaneId,
        geometry: PaneGeometry,
        focused: bool,
        cx: &Cx,
        frame: &mut Frame,
    );
    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame);
    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Self::Cmd>;
    fn run(
        &mut self,
        command: crate::kernel::command::CommandId,
        cx: &Cx,
    ) -> Vec<(PluginId, Out<Self::Cmd>)>;
    fn update(&mut self, msg: Self::Msg, cx: &Cx) -> Vec<(PluginId, Out<Self::Cmd>)>;
    fn on_event(
        &mut self,
        to: EventTarget,
        event: &Event,
        cx: &Cx,
    ) -> Vec<(PluginId, Out<Self::Cmd>)>;
    fn search(&mut self, needle: &str, cx: &Cx) -> Vec<SearchMatch>;
    fn after_batch(&mut self, pending: &[Self::Cmd], cx: &Cx) -> Vec<(PluginId, Out<Self::Cmd>)>;
    fn settle(&mut self, cx: &Cx);
    fn perform(engine: &Engine, cmd: Self::Cmd) -> Self::Msg;
    fn startup_failure(msg: Self::Msg) -> Result<Self::Msg, Box<EngineError>>;
}
