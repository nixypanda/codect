//! Read-only Git access for OwnAI.
//!
//! This crate owns repository discovery, revision resolution, commit peeling,
//! tree traversal, and blob reads. It presents OwnAI-owned values through the
//! `SnapshotRepository` interface.
//!
//! `gix` types must never leave this crate.

pub mod repository;
pub mod revision;
pub mod tree;
