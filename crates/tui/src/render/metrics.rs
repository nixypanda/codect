//! The responsive width constants shared by the layout, the shell, and the view.
//!
//! These live below the app so layout arithmetic can read them without reaching
//! upward.

pub(crate) const SINGLE_PANE_MIN_WIDTH: u16 = 40;

pub(crate) const MIN_HEIGHT: u16 = 8;

pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;
