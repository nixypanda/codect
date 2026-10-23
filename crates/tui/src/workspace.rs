//! The workspace vocabulary, re-exported from the kernel.
//!
//! The definitions live in [`tui_core::workspace`] and [`tui_core::search`];
//! the built-in plugins read them here.

#[allow(unused_imports)]
pub(crate) use tui_core::search::{Search, SearchMatch, SearchPane, collect_matches};
#[allow(unused_imports)]
pub(crate) use tui_core::workspace::{
    BadgeTone, EmptyText, FileSet, NO_CHANGES, NO_FILES, Query, Workspace,
};
