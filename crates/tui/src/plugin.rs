//! The pane vocabulary: plugin, pane, and focus identifiers.
//!
//! These are plain values. The layout, the focus ring, and the pages use them
//! to name a region without naming a type; a `PaneId`'s owner is the plugin
//! that draws it, and several panes may share one `FocusKey` (the diff's two
//! sides do). They are `pub` so the bench-feature façade stays clean, as with
//! [`crate::workspace`].

/// A plugin's stable identity. `PluginId("kernel")` is reserved.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PluginId(pub &'static str);

/// One pane a plugin owns. `slot` distinguishes the panes of one owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PaneId {
    pub owner: PluginId,
    pub slot: u8,
}

/// The unit focus moves between. Panes that scroll together share one.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct FocusKey {
    pub owner: PluginId,
    pub slot: u8,
}

/// The tree plugin's id.
pub(crate) const TREE: PluginId = PluginId("tree");

/// The source view plugin's id.
pub(crate) const SOURCE: PluginId = PluginId("source");

/// The diff plugin's id.
pub(crate) const DIFF: PluginId = PluginId("diff");

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
