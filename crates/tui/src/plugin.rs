//! The pane vocabulary: plugin, pane, and focus identifiers.
//!
//! These are plain values. The layout, the focus ring, and the pages use them
//! to name a region without naming a type; a `PaneId`'s owner is the plugin
//! that draws it, and several panes may share one `FocusKey` (the diff's two
//! sides do).
//!
//! The identifier types live in [`crate::kernel::plugin`]; this module
//! re-exports them and holds the built-in pane constants until Phase 5.

pub(crate) use crate::kernel::plugin::{FocusKey, PaneId, PluginId, ViewId};

/// The tree plugin's id.
pub(crate) const TREE: PluginId = PluginId("tree");

/// The source view plugin's id.
pub(crate) const SOURCE: PluginId = PluginId("source");

/// The diff plugin's id.
pub(crate) const DIFF: PluginId = PluginId("diff");

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

/// The base side of a diff: `Body`.
pub(crate) const OLD_PANE: PaneId = PaneId {
    owner: DIFF,
    slot: 0,
};

/// The target side of a diff: `Body`.
pub(crate) const NEW_PANE: PaneId = PaneId {
    owner: DIFF,
    slot: 1,
};

/// The commits band: `Band`.
pub(crate) const COMMITS_PANE: PaneId = PaneId {
    owner: DIFF,
    slot: 2,
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

/// The diff body's focus key, shared by both sides.
pub(crate) const DIFF_BODY_FOCUS: FocusKey = FocusKey {
    owner: DIFF,
    slot: 0,
};

/// The commits band's focus key.
pub(crate) const DIFF_BAND_FOCUS: FocusKey = FocusKey {
    owner: DIFF,
    slot: 1,
};

/// The source view's only view.
pub(crate) const SOURCE_VIEW: ViewId = ViewId {
    owner: SOURCE,
    slot: 0,
};

/// The diff plugin's compare view.
pub(crate) const COMPARE_VIEW: ViewId = ViewId {
    owner: DIFF,
    slot: 0,
};

/// The diff plugin's history view.
pub(crate) const HISTORY_VIEW: ViewId = ViewId {
    owner: DIFF,
    slot: 1,
};
