//! The TUI kernel: the terminal runtime, the frame, the shared state, the
//! plugin contract, and the design system.
//!
//! This crate is the bottom of the terminal frontend. It owns the run loop,
//! the terminal lifecycle, the layout, the focus ring, the overlay slot, the
//! command registry, the workspace, and the drawing primitives. `tui` and
//! `tui-diff` build against it; it depends on neither and never names a
//! plugin: no plugin type, plugin id string, or plugin module appears here.
//! The identifiers a plugin uses to name its panes (`PluginId`, `PaneId`,
//! `FocusKey`, `ViewId`) are plain values, not plugin knowledge.
//!
//! # Boundaries
//!
//! - No `clap` or `miette`. Argument parsing and diagnostic reporting belong to
//!   the command line.
//! - The crate never discovers a repository or reads `.codect.toml`; it receives
//!   an [`engine::Engine`] and a fully-built selection.
//! - Engine calls are effects: they are described by a plugin's `Cmd` and only
//!   ever run in the runtime, never inside `update` or the render path.
//! - No plugin crate depends on this one the other way: `tui-core` links
//!   neither `tui` nor `tui-diff`, checked by `just check-tui-core-boundary`.
//!
//! # Modules
//!
//! Every module is public because the two plugin crates are separate; each
//! module's doc comment names the audience. Nothing here is a stability
//! promise: the contract is an unstable internal one, re-exported by `tui`.

pub mod app;
pub mod chrome;
pub mod command;
pub mod cx;
pub mod dispatch;
pub mod focus;
pub mod help;
pub mod keymap;
pub mod layout;
pub mod nav;
pub mod plugin;
pub mod plugin_set;
pub mod render;
pub mod runtime;
pub mod search;
pub mod util;
pub mod view;
pub mod widgets;
pub mod workspace;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

#[cfg(test)]
mod kernel_tests;

pub use runtime::{TuiError, run};
