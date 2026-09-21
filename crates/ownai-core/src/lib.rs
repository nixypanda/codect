//! Language-agnostic core for OwnAI.
//!
//! Must not depend on Git, a parser, a grammar, or the CLI; language-specific
//! Tree-sitter node names and `gix` types must never appear here.

pub mod diagnostic;
pub mod diff;
pub mod language;
pub mod model;
pub mod path;
pub mod render;

pub use diagnostic::{
    Diagnostic, DiagnosticContext, ProjectionError, RepoPathError, decode_source,
};
pub use diff::unified_hunks;
pub use language::{LanguageProjector, ProjectionInput};
pub use model::{
    ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath, SourceSpan,
};
pub use path::{Area, AreaError, AreaSet, PathScope, PathSelection, PathSelectionError};
pub use render::{diff_document, project_all, select_projector, show_document, sort_files};
