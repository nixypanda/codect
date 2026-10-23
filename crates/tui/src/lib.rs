//! The terminal frontend.
//!
//! This crate owns the built-in plugins and the registry; the kernel, the
//! plugin contract, and the design system live in `tui-core`, and the compare
//! and history views in `tui-diff` (behind the default-on `diff` feature). The
//! public `run` builds the registry and hands it to the kernel runtime.
//!
//! # Boundaries
//!
//! - No `clap` or `miette`. Argument parsing and diagnostic reporting belong to
//!   the command line.
//! - The crate never discovers a repository or reads `.codect.toml`; it receives
//!   an [`Engine`] and a fully-built selection.
//! - Engine calls are effects: they are described per plugin and only ever run
//!   in the runtime, never inside `update` or the render path.

use std::io::IsTerminal;

use engine::Engine;

pub use tui_core::TuiError;
pub use tui_core::render::icons::IconStyle;

#[cfg(feature = "diff")]
pub use route::DiffView;
pub use route::LoadRequest;
use tui_core::render::theme::Theme;

#[cfg(test)]
mod app_tests;
mod finder;
#[cfg(test)]
mod golden;
mod help;
mod mode_picker;
mod palette;
mod plugin;
mod plugins;
mod prompt;
mod route;
mod scope_chooser;
mod search_input;
mod source;
#[cfg(test)]
pub(crate) mod test_support;
mod tree;
mod which_key;
mod workspace;

#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench;

/// The options a session starts with.
#[derive(Clone, Debug)]
pub struct TuiOptions {
    pub request: LoadRequest,
    pub scope_label: String,
    pub icons: IconStyle,
}

pub fn run(engine: Engine, options: TuiOptions) -> Result<(), TuiError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal);
    }
    tui_core::runtime::install_panic_hook();
    // Query the terminal's background before the event reader is first polled:
    // `terminal-colorsaurus` reads the reply from standard input itself, and the
    // query briefly consumes any keystroke typed during startup.
    let theme = Theme::detect();
    let (app, startup) = plugins::build(plugins::KernelOptions {
        root: engine.root().display().to_string(),
        request: options.request,
        scope_label: options.scope_label,
        width: 0,
        height: 0,
        theme,
        icons: tui_core::render::icons::Icons::new(options.icons),
    });
    tui_core::run(&engine, app, startup)
}
