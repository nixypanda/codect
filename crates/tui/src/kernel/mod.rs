//! The kernel: shared state, the plugin contract, layout, chrome, and the
//! runtime. It never names a plugin.
//!
//! In this phase the crate is still one; the crate split in Phase 5 moves these
//! modules into `tui-core`.

pub(crate) mod app;
pub(crate) mod chrome;
pub(crate) mod command;
pub(crate) mod cx;
pub(crate) mod dispatch;
pub(crate) mod focus;
pub(crate) mod help;
pub(crate) mod keymap;
pub(crate) mod layout;
pub(crate) mod nav;
pub(crate) mod plugin;
pub(crate) mod plugin_set;
pub(crate) mod runtime;
pub(crate) mod search;
pub(crate) mod view;
pub(crate) mod workspace;
