//! The route: what a session opens with, and the concrete requests derived from
//! it.
//!
//! This is the Elm SPA's `Route.elm`. A session is pinned to exactly one kind of
//! projection — `show` or `diff` — and never switches. The command line builds a
//! [`LoadRequest`]; the frontend turns the current page state into
//! [`ShowRequest`] or [`DiffRequest`] values the engine can run. A diff
//! request's `merge_base` flag only applies to the range view.

use base::{ProjectionMode, Selection};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadRequest {
    Show {
        revision: String,
        mode: ProjectionMode,
        selection: Selection,
    },
    Diff {
        base: String,
        target: String,
        mode: ProjectionMode,
        selection: Selection,
        view: DiffView,
        merge_base: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffView {
    Range,
    Commits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShowRequest {
    pub revision: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRequest {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
    pub view: DiffView,
    pub merge_base: bool,
}
