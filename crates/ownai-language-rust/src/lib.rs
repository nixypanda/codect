//! Rust language adapter for OwnAI.
//!
//! Owns the Rust parser constructor and all Rust grammar node-kind knowledge,
//! and implements the OwnAI language projector interface for Rust source files.

pub mod extract;
pub mod render;
pub mod syntax;
