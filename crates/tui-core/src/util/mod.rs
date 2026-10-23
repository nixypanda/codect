//! Small shared leaves with no dependency on the UI: the bounded cache, the
//! fuzzy matcher, and the key/mouse vocabulary.
//!
//! `tui` and `tui-diff` may use these; nothing here reads the kernel.

pub mod cache;
pub mod fuzzy;
pub mod input;
