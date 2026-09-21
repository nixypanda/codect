//! Language-agnostic core for OwnAI.
//!
//! This crate owns the shared projection model, the language adapter
//! interface, canonical rendering of project and file framing, the textual diff
//! engine, and typed diagnostics.
//!
//! It must not depend on Git, a parser, a grammar, or the CLI. Language-specific
//! Tree-sitter node names and `gix` types must never appear here.

pub mod diagnostic;
pub mod diff;
pub mod language;
pub mod model;
pub mod render;

pub use diagnostic::{
    Diagnostic, DiagnosticContext, ProjectionError, RepoPathError, decode_source,
};
pub use language::{LanguageProjector, ProjectionInput};
pub use model::{
    ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath, SourceSpan,
};
