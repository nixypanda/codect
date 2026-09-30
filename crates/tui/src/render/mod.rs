//! Presentation support: the design tokens, geometry, and primitives every
//! other layer draws with.
//!
//! This is the bottom of the frontend: nothing here reaches upward into a page,
//! a component, the shell, or the runtime. `theme`, `icons`, `layout`, `text`,
//! and `highlight` resolve styling and geometry; `block` and `empty` are the
//! shared pane and popup chrome that components and pages both draw inside.
//! `metrics` owns [`Focus`] and the responsive width constants, which were
//! previously app state and forced this layer to depend upward.

pub(crate) mod block;
pub(crate) mod empty;
pub(crate) mod highlight;
pub(crate) mod icons;
pub(crate) mod layout;
pub(crate) mod metrics;
pub(crate) mod text;
pub(crate) mod theme;
