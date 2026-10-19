//! The `#[doc(hidden)]` row-layout hook for the benchmark façade.
//!
//! This exists so `tui`'s `bench` module can measure the exact wrapped-row
//! computation a frame performs without widening the plugin's surface. It is
//! compiled only with the crate's `bench` feature and is not part of any
//! supported API.

use tui_core::cx::Cx;

use crate::{DiffPlugin, VisualRow};

/// The wrapped diff rows for the plugin's active file at the current layout.
pub fn compute_rows(plugin: &DiffPlugin, cx: &Cx) -> Vec<VisualRow> {
    plugin.compute_rows(cx)
}
