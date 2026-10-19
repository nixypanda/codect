//! The route: what a session opens with.
//!
//! This is the Elm SPA's `Route.elm`. A session is pinned to exactly one kind of
//! projection — `show` or `diff` — and never switches. The command line builds a
//! [`LoadRequest`]; the frontend turns the current view state into a
//! [`ShowRequest`] or [`DiffRequest`] value the engine can run. A diff request's
//! `merge_base` flag only applies to the range view.

use base::{ProjectionMode, Selection};

pub use crate::source::ShowRequest;
#[cfg(feature = "diff")]
pub use tui_diff::{DiffRequest, DiffView};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadRequest {
    Show {
        revision: String,
        mode: ProjectionMode,
        selection: Selection,
    },
    #[cfg(feature = "diff")]
    Diff {
        base: String,
        target: String,
        mode: ProjectionMode,
        selection: Selection,
        view: DiffView,
        merge_base: bool,
    },
}
