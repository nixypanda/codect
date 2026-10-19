//! The pane vocabulary: plugin, pane, focus, and view identifiers.
//!
//! These are plain values. The layout, the focus ring, and the plugins use them
//! to name a region without naming a type; a `PaneId`'s owner is the plugin
//! that draws it, and several panes may share one `FocusKey` (the diff's two
//! sides do). The identifier types come from [`tui_core::plugin`]; the
//! constants here are the built-in plugins' addresses.

pub(crate) use tui_core::plugin::{FocusKey, PaneId, PluginId, ViewId};

/// The tree plugin's id.
pub(crate) const TREE: PluginId = PluginId("tree");

/// The source view plugin's id.
pub(crate) const SOURCE: PluginId = PluginId("source");

/// The which-key band plugin's id.
pub(crate) const WHICH_KEY: PluginId = PluginId("which-key");

/// The command palette plugin's id.
pub(crate) const PALETTE: PluginId = PluginId("palette");

/// The help plugin's id.
pub(crate) const HELP: PluginId = PluginId("help");

/// The file finder plugin's id.
pub(crate) const FINDER: PluginId = PluginId("finder");

/// The mode picker plugin's id.
pub(crate) const MODES: PluginId = PluginId("modes");

/// The scope chooser plugin's id.
pub(crate) const SCOPE: PluginId = PluginId("scope");

/// The search input plugin's id.
pub(crate) const SEARCH: PluginId = PluginId("search");

/// The text prompt plugin's id.
pub(crate) const PROMPT: PluginId = PluginId("prompt");

/// The file tree: `Navigator`.
pub(crate) const TREE_PANE: PaneId = PaneId {
    owner: TREE,
    slot: 0,
};

/// The source body: `Body`.
pub(crate) const SOURCE_PANE: PaneId = PaneId {
    owner: SOURCE,
    slot: 0,
};

/// The which-key band's overlay pane.
pub(crate) const WHICH_KEY_PANE: PaneId = PaneId {
    owner: WHICH_KEY,
    slot: 0,
};

/// The command palette's overlay pane.
pub(crate) const PALETTE_PANE: PaneId = PaneId {
    owner: PALETTE,
    slot: 0,
};

/// The help popup's overlay pane.
pub(crate) const HELP_PANE: PaneId = PaneId {
    owner: HELP,
    slot: 0,
};

/// The file finder's overlay pane.
pub(crate) const FINDER_PANE: PaneId = PaneId {
    owner: FINDER,
    slot: 0,
};

/// The mode picker's overlay pane.
pub(crate) const MODES_PANE: PaneId = PaneId {
    owner: MODES,
    slot: 0,
};

/// The scope chooser's overlay pane.
pub(crate) const SCOPE_PANE: PaneId = PaneId {
    owner: SCOPE,
    slot: 0,
};

/// The search input's overlay pane.
pub(crate) const SEARCH_PANE: PaneId = PaneId {
    owner: SEARCH,
    slot: 0,
};

/// The revision prompt's overlay pane.
pub(crate) const PROMPT_PANE: PaneId = PaneId {
    owner: PROMPT,
    slot: 0,
};

/// The file tree's focus key.
pub(crate) const TREE_FOCUS: FocusKey = FocusKey {
    owner: TREE,
    slot: 0,
};

/// The source body's focus key.
pub(crate) const SOURCE_FOCUS: FocusKey = FocusKey {
    owner: SOURCE,
    slot: 0,
};

/// The source view's only view.
pub(crate) const SOURCE_VIEW: ViewId = ViewId {
    owner: SOURCE,
    slot: 0,
};

#[cfg(feature = "diff")]
#[allow(unused_imports)]
pub(crate) use tui_diff::{
    COMMITS_PANE, COMPARE_VIEW, DIFF, DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, HISTORY_VIEW, NEW_PANE,
    OLD_PANE,
};
