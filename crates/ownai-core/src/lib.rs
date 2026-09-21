//! Language-agnostic core for OwnAI.
//!
//! Must not depend on Git, a parser, a grammar, or the CLI; language-specific
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
