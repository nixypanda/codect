//! The isolated components: each owns its state, its messages, its update, and
//! its rendering.
//!
//! A component never names [`crate::app::App`]. Rendering reads only the narrow
//! [`RenderCtx`] the parent builds, and the pane primitives come from
//! [`crate::render`], so a component stays self-contained and every dependency
//! points downward: `render` and `util` below, app and view above.

pub(crate) mod commit_picker;
pub(crate) mod overlay;
pub(crate) mod text_input;
pub(crate) mod tree;

use crate::page::Loaded;
use crate::render::icons::Icons;
use crate::render::theme::Theme;

pub(crate) struct RenderCtx<'a> {
    pub theme: &'a Theme,
    pub icons: &'a Icons,
    pub loaded: &'a Loaded,
    pub busy: bool,
}
