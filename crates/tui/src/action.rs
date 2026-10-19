//! Semantic commands, split so that actions only valid in one content kind do
//! not exist in the others.
//!
//! Keys and the command palette both produce these values, so a binding and its
//! palette entry cannot drift apart. A [`GlobalAction`] is valid whatever is
//! loaded; the others are scoped to a variant and only offered by that variant's
//! palette.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalAction {
    Quit,
    Help,
    TreeWider,
    TreeNarrower,
    TreeReset,
    Top,
    Bottom,
    PageUp,
    PageDown,
    NextMatch,
    PreviousMatch,
    NextPane,
    PreviousPane,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowAction {
    EditRevision,
    ToggleFold,
    OpenFold,
    CloseFold,
    OpenRecursive,
    CloseRecursive,
    OpenAll,
    CloseAll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeAction {
    EditBase,
    EditTarget,
    SwitchToCommits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitsAction {
    SwitchToRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Global(GlobalAction),
    Show(ShowAction),
    Range(RangeAction),
    Commits(CommitsAction),
}

/// A binding under a prefix. `hint` is the full keystroke the palette shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding {
    pub key: char,
    pub action: Action,
    pub label: &'static str,
    pub hint: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixGroup {
    pub key: char,
    pub name: &'static str,
    pub bindings: Vec<Binding>,
}
