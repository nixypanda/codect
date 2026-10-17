//! The focus ring: the order focus moves through the visible panes.

use crate::kernel::layout::{PaneSpec, Placement};
use crate::kernel::plugin::FocusKey;

/// The focus ring: the distinct focus keys of `panes`, bands first, then
/// navigators, then bodies. Within a placement, registration order, then first
/// appearance. Panes that share a focus key are one stop.
pub fn focus_ring(panes: &[PaneSpec]) -> Vec<FocusKey> {
    let mut ring: Vec<FocusKey> = Vec::new();
    for placement in [Placement::Band, Placement::Navigator, Placement::Body] {
        for spec in panes.iter().filter(|spec| spec.placement == placement) {
            if !ring.contains(&spec.focus) {
                ring.push(spec.focus);
            }
        }
    }
    ring
}

/// The default focus: the first navigator's focus key, else the first body's.
pub fn focus_default(panes: &[PaneSpec]) -> Option<FocusKey> {
    panes
        .iter()
        .find(|spec| spec.placement == Placement::Navigator)
        .or_else(|| panes.iter().find(|spec| spec.placement == Placement::Body))
        .map(|spec| spec.focus)
}

/// The first pane whose focus key is `focus`, for routing pane input.
pub fn first_pane_with_focus(
    panes: &[PaneSpec],
    focus: FocusKey,
) -> Option<crate::kernel::plugin::PaneId> {
    panes
        .iter()
        .find(|spec| spec.focus == focus)
        .map(|spec| spec.id)
}
