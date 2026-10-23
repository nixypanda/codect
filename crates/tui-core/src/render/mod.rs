//! Presentation support: the design tokens and the drawing primitives the
//! kernel and every plugin draw with.
//!
//! This is the crate's bottom layer: nothing here reaches upward into the
//! kernel or a plugin. `theme`, `icons`, `text`, and `highlight` resolve
//! styling; `block` and `empty` are the shared pane and popup chrome. The
//! frame and body arithmetic lives in [`crate::layout`], and the shared list and
//! input widgets in [`crate::widgets`]. `tui` and `tui-diff` may use all of it.

pub mod block;
pub mod empty;
pub mod highlight;
pub mod icons;
pub mod text;
pub mod theme;
