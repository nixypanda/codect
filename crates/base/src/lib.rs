//! Language-agnostic core for OwnAI.
//!
//! Must not depend on Git, a parser, a grammar, or the CLI; language-specific
//! Tree-sitter node names and `gix` types must never appear here.

pub mod diagnostic;
pub mod diff;
pub mod doc;
pub mod keys;
pub mod language;
pub mod model;
pub mod outline;
pub mod path;
pub mod render;
pub mod selection;

pub use diagnostic::{
    Diagnostic, DiagnosticContext, Location, ProjectionError, RepoPathError, decode_source,
};
pub use diff::{AlignedRow, DiffLine, DiffRowKind, FileDiff, aligned_rows, unified_hunks};
pub use doc::Doc;
pub use keys::KeyAllocator;
pub use language::{LanguageProjector, ProjectionInput};
pub use model::{
    ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath, SourceSpan,
    SupportedPath,
};
pub use outline::{FileOutline, FileOutlineDiff, OutlineItem, assemble_outline};
pub use path::{Area, AreaError, AreaSet, PathScope, PathSelection};
pub use render::{
    LINE_WIDTH, diff_document, project_all, select_projector, show_document, sort_files,
};
pub use selection::{Selection, SelectionError, SelectionGroup};
