//! The responsive thresholds and the normalized pane focus.
//!
//! These live below the app so layout arithmetic can read them without reaching
//! upward. [`Focus`] is the layout's vocabulary for "which pane is active"; a
//! page maps its own richer focus enum onto it.

/// Below this width the terminal shows a single pane.
pub(crate) const SINGLE_PANE_MIN_WIDTH: u16 = 40;

/// Below this height the terminal is too small to draw.
pub(crate) const MIN_HEIGHT: u16 = 8;

/// At or above this width the tree and content render side by side.
pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

/// Which body pane has focus, normalized for layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Focus {
    Commits,
    Tree,
    Content,
}
