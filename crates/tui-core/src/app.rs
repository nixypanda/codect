//! The kernel: shared state, the one update step, and settle.
//!
//! `App<S>` pairs the kernel state with the plugin set. `update` clones, folds
//! one message, and returns the replacement plus the effects to run. The
//! request cascade and key/mouse dispatch live in [`crate::dispatch`]. Every TUI crate
//! builds an [`App`] and drives it with [`update`] and [`settle`].

#![allow(dead_code)]
use crate::command::CommandSpec;
use crate::cx::Cx;
use crate::dispatch;
use crate::help::HelpEntry;
use crate::keymap::Keymap;
use crate::layout::PaneSpec;
use crate::plugin::{FocusKey, PaneId, PluginSet, Size, ViewId};
use crate::render::icons::Icons;
use crate::render::theme::Theme;
use crate::util::input::{Key, Mouse};
use crate::workspace::Workspace;

const TREE_DEFAULT_PERCENT: u16 = 30;

const DIAGNOSTIC_TICKS: u8 = 40;

/// The kernel's shared state.
#[derive(Clone)]
pub struct Kernel {
    pub root: String,
    pub size: Size,
    pub chrome: Chrome,
    pub workspace: Workspace,
    pub focus: Option<FocusKey>,
    pub active_view: ViewId,
    pub overlay: Option<PaneId>,
    pub keymap: Keymap,
    pub diagnostic: Option<Diagnostic>,
}

/// The frame state the kernel owns: the navigator width, the theme, the icons,
/// the spinner, and the quit flag.
#[derive(Clone)]
pub struct Chrome {
    pub tree_percent: u16,
    pub theme: Theme,
    pub icons: Icons,
    pub spinner: u8,
    pub quit: bool,
}

/// A timed diagnostic toast.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub text: String,
    pub ttl: u8,
}

impl Diagnostic {
    pub fn new(text: String) -> Self {
        Self {
            text,
            ttl: DIAGNOSTIC_TICKS,
        }
    }
}

/// The kernel message: input, life-cycle, and one plugin's effect result.
#[derive(Debug)]
pub enum Msg<M> {
    Key(Key),
    Mouse(Mouse),
    Resize { width: u16, height: u16 },
    Tick,
    Plugin(M),
}

/// The application: kernel state plus the installed plugins.
#[derive(Clone)]
pub struct App<S: PluginSet> {
    pub kernel: Kernel,
    pub plugins: S,
}

impl<S: PluginSet> App<S> {
    pub fn new(kernel: Kernel, plugins: S) -> Self {
        Self { kernel, plugins }
    }

    /// Whether any plugin is loading. Convenience for the shell and tests.
    pub fn is_busy(&self) -> bool {
        self.plugins.busy()
    }
}

impl<S: PluginSet> std::ops::Deref for App<S> {
    type Target = Kernel;

    fn deref(&self) -> &Kernel {
        &self.kernel
    }
}

impl<S: PluginSet> std::ops::DerefMut for App<S> {
    fn deref_mut(&mut self) -> &mut Kernel {
        &mut self.kernel
    }
}

/// The declarations collected once per update step.
pub struct Specs {
    pub panes: Vec<PaneSpec>,
    pub commands: Vec<CommandSpec>,
    pub help: Vec<HelpEntry>,
}

impl Specs {
    /// Collect the declarations, always asking for the panes but only building
    /// the command registry and help when `commands`/`help` are set. The body
    /// layout and focus routing need only the panes; the command registry is
    /// built lazily because it allocates a spec and a binding vector per
    /// command on every call.
    fn collect<S: PluginSet>(app: &App<S>, commands: bool, help: bool) -> Self {
        let kernel = &app.kernel;
        let overlay = app.kernel.overlay;
        let bootstrap = Cx::new(
            kernel.size,
            &kernel.chrome.theme,
            &kernel.chrome.icons,
            &kernel.root,
            &kernel.workspace,
            kernel.focus,
            kernel.active_view,
            overlay,
            kernel.keymap.pending,
            app.plugins.busy(),
            kernel.chrome.tree_percent,
            kernel.diagnostic.is_some(),
            &[],
            &[],
            &[],
        );
        let panes = app.plugins.panes(&bootstrap);
        let commands = if commands {
            let mut specs = crate::command::kernel_commands();
            specs.extend(app.plugins.commands(&bootstrap));
            #[cfg(debug_assertions)]
            if let Err(message) = crate::command::validate(&specs, kernel.focus) {
                panic!("command registry validation failed: {message}");
            }
            specs
        } else {
            Vec::new()
        };
        let help = if help {
            app.plugins.help(&bootstrap)
        } else {
            Vec::new()
        };
        Specs {
            panes,
            commands,
            help,
        }
    }

    /// Only the panes: enough for the body layout, focus routing, and resize.
    pub fn panes_only<S: PluginSet>(app: &App<S>) -> Self {
        Self::collect(app, false, false)
    }

    /// Every declaration: the panes plus the command registry and help.
    pub fn full<S: PluginSet>(app: &App<S>) -> Self {
        Self::collect(app, true, true)
    }
}

/// Collect `panes`, `commands`, and `help` through a bootstrap `Cx`.
pub fn collect_specs<S: PluginSet>(app: &App<S>) -> Specs {
    Specs::full(app)
}

/// Build the real `Cx` from kernel state and the collected declarations.
pub fn cx_for<'a>(
    kernel: &'a Kernel,
    overlay: Option<PaneId>,
    busy: bool,
    specs: &'a Specs,
) -> Cx<'a> {
    Cx::new(
        kernel.size,
        &kernel.chrome.theme,
        &kernel.chrome.icons,
        &kernel.root,
        &kernel.workspace,
        kernel.focus,
        kernel.active_view,
        overlay,
        kernel.keymap.pending,
        busy,
        kernel.chrome.tree_percent,
        kernel.diagnostic.is_some(),
        &specs.panes,
        &specs.commands,
        &specs.help,
    )
}

pub fn update<S: PluginSet>(msg: Msg<S::Msg>, app: &App<S>) -> (App<S>, Vec<S::Cmd>) {
    let mut next = app.clone();
    let mut cmds = Vec::new();

    match msg {
        Msg::Key(key) => {
            let specs = Specs::full(&next);
            dispatch::dispatch_key(&mut next, key, &mut cmds, &specs);
        }
        Msg::Mouse(event) => {
            let specs = Specs::full(&next);
            dispatch::dispatch_mouse(&mut next, event, &mut cmds, &specs);
        }
        Msg::Resize { width, height } => {
            next.kernel.size = Size { width, height };
            let specs = Specs::panes_only(&next);
            dispatch::broadcast(
                &mut next,
                crate::plugin::Event::Resized,
                &mut cmds,
                0,
                &specs,
            );
        }
        Msg::Tick => {
            next.kernel.chrome.spinner = next.kernel.chrome.spinner.wrapping_add(1);
            if let Some(diagnostic) = &mut next.kernel.diagnostic {
                diagnostic.ttl = diagnostic.ttl.saturating_sub(1);
                if diagnostic.ttl == 0 {
                    next.kernel.diagnostic = None;
                }
            }
        }
        Msg::Plugin(msg) => {
            let specs = Specs::full(&next);
            let outs = {
                let kernel = &next.kernel;
                let overlay = next.kernel.overlay;
                let busy = next.plugins.busy();
                let cx = cx_for(kernel, overlay, busy, &specs);
                next.plugins.update(msg, &cx)
            };
            dispatch::process_wrapped(&mut next, outs, &mut cmds, 0, &specs);
        }
    }

    normalize_focus(&mut next);
    (next, cmds)
}

/// Focus normalisation (section 4.7 step 6): a focus that names no current pane
/// falls back to the first focus stop. This runs on every update step, so it
/// asks the plugins for membership and the fallback directly instead of
/// collecting the panes.
fn normalize_focus<S: PluginSet>(app: &mut App<S>) {
    let current = app.kernel.focus;
    if !current.is_some_and(|focus| app.plugins.has_focus(focus)) {
        app.kernel.focus = app.plugins.first_focus();
    }
}

/// Run the settle phase: plugin rows and highlighting, then the kernel's search
/// re-sync.
pub fn settle<S: PluginSet>(mut app: App<S>) -> App<S> {
    let specs = Specs::panes_only(&app);
    {
        let kernel = &app.kernel;
        let overlay = app.kernel.overlay;
        let busy = app.plugins.busy();
        let cx = cx_for(kernel, overlay, busy, &specs);
        app.plugins.settle(&cx);
    }
    sync_search(&mut app, &specs);
    app
}

fn sync_search<S: PluginSet>(app: &mut App<S>, specs: &Specs) {
    let Some(needle) = app
        .kernel
        .workspace
        .search
        .as_ref()
        .map(|search| search.needle.clone())
    else {
        return;
    };
    let matches = {
        let kernel = &app.kernel;
        let overlay = app.kernel.overlay;
        let busy = app.plugins.busy();
        let cx = cx_for(kernel, overlay, busy, specs);
        app.plugins.search(&needle, &cx)
    };
    if let Some(search) = app.kernel.workspace.search.as_mut() {
        search.matches = matches;
        search.cursor = search.cursor.min(search.matches.len().saturating_sub(1));
    }
}
