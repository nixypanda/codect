//! Semantic commands, split so that actions only valid in one content kind do
//! not exist in the others.
//!
//! Keys and the command palette both produce these values, so a binding and its
//! palette entry cannot drift apart. A [`GlobalAction`] is valid whatever is
//! loaded; the others are scoped to a variant and only offered by that variant's
//! palette.

/// A semantic command valid in every content kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalAction {
    Quit,
    Help,
    Scope,
    Mode,
    TreeWider,
    TreeNarrower,
    TreeReset,
    Top,
    Bottom,
    PageUp,
    PageDown,
    Palette,
    Finder,
    Search,
    NextMatch,
    PreviousMatch,
    NextPane,
    PreviousPane,
}

/// A semantic command valid only while a `show` projection is loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowAction {
    EditRevision,
}

/// A semantic command valid only in the `diff` range view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeAction {
    EditBase,
    EditTarget,
    SwitchToCommits,
}

/// A semantic command valid only in the `diff` commits view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitsAction {
    SwitchToRange,
}

/// A semantic command produced by a key or the command palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Global(GlobalAction),
    Show(ShowAction),
    Range(RangeAction),
    Commits(CommitsAction),
}
