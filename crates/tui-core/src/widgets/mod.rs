//! Shared widgets: the single-line text input and the fuzzy-ranked list.
//!
//! The widgets own only their arguments and the theme; the overlays in `tui`
//! own the surrounding state. `tui` may use these; `tui-diff` does not.

pub mod ranked_list;
pub mod text_input;
