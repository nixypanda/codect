//! The shared application layer over Git, the core projection model, and the
//! language adapters.
//!
//! `ownai-engine` is the Git-aware pipeline both the command line and the
//! terminal frontend run. It discovers repositories, resolves revisions,
//! validates a [`Selection`], and projects committed blobs; it never renders a
//! document and never reads command-line arguments.
//!
//! # Boundaries
//!
//! - No `clap`, `miette`, `ratatui`, or `crossterm` dependency. Presenting a
//!   diagnostic belongs to the caller.
//! - Every public result is an OwnAI-owned value or a typed [`EngineError`];
//!   `gix` types stay inside `ownai-git`.
//! - Caches are per-operation. An [`Engine`] holds no unbounded session state.

pub mod config;
pub mod engine;
pub mod error;
pub mod item_diff;
pub mod review;
pub mod selection;

pub use engine::{Engine, FileDiff};
pub use error::EngineError;
pub use item_diff::{ItemChangeKind, ItemDiff};
pub use review::{
    ChoiceJudgment, REVIEW_STATE_SCHEMA, ReviewError, ReviewJudgment, ReviewLens, ReviewOutcome,
    ReviewedItemDiff, ScoreJudgment,
};
pub use selection::{Selection, SelectionError, SelectionGroup};
