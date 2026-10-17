//! The terminal frontend.
//!
//! This crate is the only layer that touches the terminal. The kernel
//! ([`kernel`]) owns the run loop, the terminal lifecycle, the frame, and the
//! shared state; every UI surface is a plugin ([`plugins`]). The public `run`
//! builds the registry and hands it to the kernel runtime.
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

pub use render::icons::IconStyle;
use render::theme::Theme;
pub use route::{DiffView, LoadRequest};

mod action;
mod api;
mod command;
mod component;
#[cfg(test)]
mod golden;
mod kernel;
mod page;
mod plugin;
mod plugins;
mod render;
mod request;
mod route;
#[cfg(test)]
pub(crate) mod test_support;
mod tree;
mod util;
mod workspace;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod kernel_tests;

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

#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("standard input and standard output must be terminals")]
    NotATerminal,

    #[error("the terminal could not be used")]
    Terminal(#[source] std::io::Error),

    #[error(transparent)]
    Engine(#[from] engine::EngineError),
}

pub fn run(engine: Engine, options: TuiOptions) -> Result<(), TuiError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal);
    }
    kernel::runtime::install_panic_hook();
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
        icons: render::icons::Icons::new(options.icons),
    });
    kernel::runtime::run(&engine, app, startup)
}
