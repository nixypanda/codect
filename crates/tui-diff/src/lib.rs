//! `tui-diff`: the compare (range) and history (commits) views.
//!
//! This crate is the optional, feature-gated `diff` plugin of the `tui` crate.
//! It depends on `tui-core` only among the TUI crates and never on `tui`: the
//! plugin is written against the kernel contract, exactly like a built-in
//! plugin.
//!
//! One plugin owns both views because they share the base, the target, the
//! merge-base flag, and the diff body; switching between them is internal. The
//! panes `OLD` and `NEW` share one focus stop; the commits band has its own.
//!
//! `model` owns the view state and caches, `plugin` the `Plugin`
//! implementation, `pane` and `commit_picker` the drawing, `layout` the wrapped
//! rows, and `emphasis` the intra-line ranges.

mod commit_picker;
mod emphasis;
mod layout;
mod model;
mod pane;
mod plugin;

#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench;

pub use commit_picker::CommitPicker;
pub use layout::VisualRow;
pub use model::DiffViewState;
pub use plugin::{Cmd, DiffPlugin, Msg};

use base::{ProjectionMode, Selection};
use tui_core::plugin::{FocusKey, PaneId, PluginId, ViewId};

/// A request to compare two revisions' projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRequest {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
    pub view: DiffView,
    pub merge_base: bool,
}

/// Which diff view a session opens with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffView {
    Range,
    Commits,
}

/// The diff plugin's id.
pub const DIFF: PluginId = PluginId("diff");

/// The base side of a diff: `Body`.
pub const OLD_PANE: PaneId = PaneId {
    owner: DIFF,
    slot: 0,
};

/// The target side of a diff: `Body`.
pub const NEW_PANE: PaneId = PaneId {
    owner: DIFF,
    slot: 1,
};

/// The commits band: `Band`.
pub const COMMITS_PANE: PaneId = PaneId {
    owner: DIFF,
    slot: 2,
};

/// The diff body's focus key, shared by both sides.
pub const DIFF_BODY_FOCUS: FocusKey = FocusKey {
    owner: DIFF,
    slot: 0,
};

/// The commits band's focus key.
pub const DIFF_BAND_FOCUS: FocusKey = FocusKey {
    owner: DIFF,
    slot: 1,
};

/// The diff plugin's compare view.
pub const COMPARE_VIEW: ViewId = ViewId {
    owner: DIFF,
    slot: 0,
};

/// The diff plugin's history view.
pub const HISTORY_VIEW: ViewId = ViewId {
    owner: DIFF,
    slot: 1,
};
