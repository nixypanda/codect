//! The frame and body layout. In this phase the implementation lives in
//! [`crate::render::layout`]; the crate split moves it here. Only the frame and
//! body arithmetic is re-exported; the diff row layout stays with the diff.

#[allow(unused_imports)]
pub(crate) use crate::render::layout::{
    BodyLayout, Edge, PaneGeometry, PaneSpec, Placement, Slot, body_layout, centered, frame_areas,
    window_offset,
};
pub(crate) use crate::render::metrics::{
    MIN_HEIGHT, SIDE_BY_SIDE_MIN_WIDTH, SINGLE_PANE_MIN_WIDTH,
};
