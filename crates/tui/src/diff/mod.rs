//! The diff plugin: the compare (range) and history (commits) views.
//!
//! The plugin will move to the optional `tui-diff` crate in Phase 5; until then
//! it lives beside the other built-in plugins. `model` owns the view state and
//! caches, `plugin` the `Plugin` implementation, `pane` and `commit_picker` the
//! drawing, `layout` the wrapped rows, and `emphasis` the intra-line ranges.

mod commit_picker;
mod emphasis;
mod layout;
mod model;
mod pane;
mod plugin;

#[allow(unused_imports)]
pub(crate) use commit_picker::CommitPicker;
#[allow(unused_imports)]
pub(crate) use layout::VisualRow;
#[allow(unused_imports)]
pub(crate) use model::DiffViewState;
#[allow(unused_imports)]
pub(crate) use plugin::{Cmd, DiffPlugin, Msg};

use base::{ProjectionMode, Selection};

/// A request to compare two revisions' projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffRequest {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub selection: Selection,
    pub view: crate::route::DiffView,
    pub merge_base: bool,
}
