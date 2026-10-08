//! The route: what a session opens with, and the concrete requests derived from
//! it.
//!
//! This is the Elm SPA's `Route.elm`. A session is pinned to exactly one kind of
//! projection — `show` or `diff` — and never switches. The command line builds a
//! [`LoadRequest`]; the frontend turns the current page state into
//! [`ShowRequest`] or [`DiffRequest`] values the engine can run.

use base::{ProjectionMode, Selection};

/// The projection to open with. A session's kind is fixed by this request.
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
        /// Compare the merge base of `base` and `target` against `target`.
        /// Only meaningful for the range view.
        merge_base: bool,
    },
}

/// Which diff view to open or reload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffView {
    /// Compare two endpoint snapshots directly. Each side may be a commit,
    /// `:index`, `:worktree`, or `:empty`.
    Range,
    /// Walk the target's first-parent chain between two commits. This needs
    /// commit sides; a snapshot endpoint is not part of a first-parent chain.
    Commits,
}

/// A concrete `show` effect request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShowRequest {
    pub revision: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
}

/// A concrete `diff` effect request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRequest {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
    pub view: DiffView,
    pub merge_base: bool,
}
