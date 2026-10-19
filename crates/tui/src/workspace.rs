//! The workspace vocabulary, re-exported from the kernel.
//!
//! The definitions live in [`crate::kernel::workspace`] and
//! [`crate::kernel::search`]; the pages and components read them here until the
//! crate split in Phase 5.

#[allow(unused_imports)]
pub(crate) use crate::kernel::search::{Search, SearchMatch, SearchPane, collect_matches};
#[allow(unused_imports)]
pub(crate) use crate::kernel::workspace::{
    BadgeTone, EmptyText, FileSet, NO_CHANGES, NO_FILES, Query, Workspace,
};
