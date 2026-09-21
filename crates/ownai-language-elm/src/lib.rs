//! Elm language adapter for OwnAI.
//!
//! Owns the Elm parser constructor and all Elm grammar node-kind knowledge, and
//! implements the OwnAI language projector interface for Elm source files.

pub mod extract;
pub mod render;
pub mod syntax;
