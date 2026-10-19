//! The isolated components: each owns its state, its messages, its update, and
//! its rendering.
//!
//! A component never names the app. Rendering reads only its arguments and the
//! pane primitives from [`crate::render`], so a component stays self-contained
//! and every dependency points downward: `render` and `util` below, the plugins
//! above.

pub(crate) mod text_input;
