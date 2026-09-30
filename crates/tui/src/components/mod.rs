//! The isolated components: each owns its state, its messages, its update, and
//! its rendering.
//!
//! A component never names [`crate::app::App`]. Rendering reads only the narrow
//! [`RenderCtx`] the parent builds, so a component stays self-contained and the
//! dependency direction — app and view point at components, never the reverse —
//! is preserved.

pub(crate) mod commit_picker;
pub(crate) mod overlay;
pub(crate) mod text_input;
pub(crate) mod tree;

use crate::content::Loaded;
use crate::icons::Icons;
use crate::theme::Theme;

/// The read-only slice of the app that a pane component's render needs.
///
/// It carries the theme and icons, whether a projection is in flight, and the
/// loaded content — enough for the tree, commits, and any future pane to draw
/// themselves without reaching into the whole app.
pub(crate) struct RenderCtx<'a> {
    pub theme: &'a Theme,
    pub icons: &'a Icons,
    pub loaded: &'a Loaded,
    pub busy: bool,
}
