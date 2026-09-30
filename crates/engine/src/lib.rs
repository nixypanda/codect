//! The shared application layer over Git, the core projection model, and the
//! language adapters.
//!
//! `engine` is the Git-aware pipeline both the command line and the
//! terminal frontend run. It discovers repositories, resolves revisions,
//! validates a selection, and projects committed blobs; it never renders a
//! document and never reads command-line arguments.
//!
//! # Boundaries
//!
//! - No `clap`, `miette`, `ratatui`, or `crossterm` dependency. Presenting a
//!   diagnostic belongs to the caller.
//! - Every public result is an Codect-owned value or a typed [`EngineError`];
//!   `gix` types stay inside `git`.
//! - Caches are per-operation. An [`Engine`] holds no unbounded session state.

pub mod config;
pub mod engine;
pub mod error;

pub use engine::{CommitDiff, Engine, SnapshotDiff, project_source};
pub use error::EngineError;
pub use git::CommitStep;
