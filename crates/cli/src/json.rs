//! The `ownai.show.v1` JSON document.
//!
//! The JSON types live in the CLI so `base` stays free of serialization
//! and `engine` stays free of file formats. The document is produced for
//! both committed revisions and editor-supplied bytes (stdin or worktree), and
//! is validated against `docs/schema/ownai.show.v1.json` by the test suite.
//!
//! # Contract highlights
//!
//! - `schema` is the literal `ownai.show.v1`; a consumer treats any other value
//!   as a fatal, explicit version mismatch.
//! - Line numbers are **one-based** for editor friendliness. Byte offsets are
//!   zero-based into the decoded UTF-8 source. The plugin converts as needed.
//! - `projection` is the requested mode; `outline` is mode-independent and
//!   complete, derived from the Signatures superset projection. For a
//!   declaration the requested mode retains, the mode-correct closed-fold text
//!   is the matching `projection.items[].canonical_text`; `outline.signature`
//!   is only the fallback for `retained_in_mode: false` and is intentionally
//!   the superset form. Container declarations (trait/impl/module) therefore
//!   differ between the two.
//! - `span` starts at the declaration node and excludes preceding attributes,
//!   decorators, `{-# ... #-}` pragmas, and doc comments, which `signature`
//!   may include. An editor extends a fold start upward over those lines; a
//!   `decorator_start_line` field can be added additively within v1 later.
//! - `stable_key` is unique within its file, not across the repository: Rust
//!   `impl` keys are not path-namespaced, so consumers key global state by
//!   `(path, stable_key)`.
//! - JSON output contains no ANSI and is unaffected by `--color`.

use base::{
    FileOutline, FileOutlineDiff, ItemKind, Language, OutlineItem, ProjectedFile, ProjectedItem,
    ProjectionMode, SourceSpan,
};
use engine::SnapshotDiff;
use serde::Serialize;

/// The schema identifier carried by every document.
pub const SCHEMA: &str = "ownai.show.v1";
pub const DIFF_SCHEMA: &str = "ownai.diff.v1";

/// Serializes changed snapshot files for a focused Diffview provider.
pub fn diff_document(
    base: &str,
    target: &str,
    mode: ProjectionMode,
    diff: &SnapshotDiff,
) -> String {
    let document = DiffDocument {
        schema: DIFF_SCHEMA,
        mode: mode_name(mode),
        base: SnapshotDocument {
            kind: diff.base_kind,
            revision: base,
            id: diff.base_id.clone(),
        },
        target: SnapshotDocument {
            kind: diff.target_kind,
            revision: target,
            id: diff.target_id.clone(),
        },
        files: diff
            .files
            .iter()
            .map(|file| diff_file_document(file, diff))
            .collect(),
    };
    let mut text =
        serde_json::to_string_pretty(&document).expect("serializing the diff document cannot fail");
    text.push('\n');
    text
}

#[derive(Serialize)]
struct DiffDocument<'a> {
    schema: &'static str,
    mode: &'static str,
    base: SnapshotDocument<'a>,
    target: SnapshotDocument<'a>,
    files: Vec<DiffFileDocument>,
}

#[derive(Serialize)]
struct SnapshotDocument<'a> {
    kind: &'static str,
    revision: &'a str,
    id: String,
}

#[derive(Serialize)]
struct DiffFileDocument {
    path: String,
    language: &'static str,
    status: &'static str,
    base: Option<DiffSideDocument>,
    target: Option<DiffSideDocument>,
    equal: bool,
}

#[derive(Serialize)]
struct DiffSideDocument {
    snapshot_id: String,
    projection: ProjectionDocument,
    outline: Vec<OutlineDocument>,
}

fn diff_file_document(file: &FileOutlineDiff, diff: &SnapshotDiff) -> DiffFileDocument {
    let (exemplar, status, base, target) = match file {
        FileOutlineDiff::Added { new } => (
            new,
            "added",
            None,
            Some(diff_side_document(new, diff.target_id.clone())),
        ),
        FileOutlineDiff::Deleted { old } => (
            old,
            "deleted",
            Some(diff_side_document(old, diff.base_id.clone())),
            None,
        ),
        FileOutlineDiff::Modified { old, new } => (
            old,
            "modified",
            Some(diff_side_document(old, diff.base_id.clone())),
            Some(diff_side_document(new, diff.target_id.clone())),
        ),
    };
    DiffFileDocument {
        path: file.path().to_string(),
        language: language_name(exemplar.language()),
        status,
        base,
        target,
        equal: false,
    }
}

fn diff_side_document(file: &FileOutline, snapshot_id: String) -> DiffSideDocument {
    DiffSideDocument {
        snapshot_id,
        projection: projection_document(&file.projection),
        outline: file
            .outline
            .iter()
            .map(|item| outline_document(file, item))
            .collect(),
    }
}

/// Which input produced a document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Input {
    /// A committed revision.
    Revision,
    /// Source bytes from standard input.
    Stdin,
    /// Source bytes read from the file at `--path` on disk.
    Worktree,
}

impl Input {
    fn name(self) -> &'static str {
        match self {
            Self::Revision => "revision",
            Self::Stdin => "stdin",
            Self::Worktree => "worktree",
        }
    }
}

/// Serializes one `ownai.show.v1` document.
///
/// The document is pretty-printed so the contract is inspectable by hand, and
/// it always ends with a single newline.
pub fn document(
    input: Input,
    revision: Option<&str>,
    mode: ProjectionMode,
    files: &[FileOutline],
) -> String {
    let document = Document {
        schema: SCHEMA,
        input: input.name(),
        revision: revision.map(str::to_owned),
        mode: mode_name(mode),
        files: files.iter().map(file_document).collect(),
    };
    let mut text =
        serde_json::to_string_pretty(&document).expect("serializing the show document cannot fail");
    text.push('\n');
    text
}

#[derive(Serialize)]
struct Document {
    schema: &'static str,
    input: &'static str,
    revision: Option<String>,
    mode: &'static str,
    files: Vec<FileDocument>,
}

#[derive(Serialize)]
struct FileDocument {
    path: String,
    language: &'static str,
    projection: ProjectionDocument,
    outline: Vec<OutlineDocument>,
}

#[derive(Serialize)]
struct ProjectionDocument {
    text: String,
    items: Vec<ItemDocument>,
}

#[derive(Serialize)]
struct ItemDocument {
    stable_key: String,
    parent_key: Option<String>,
    kind: &'static str,
    name: String,
    span: SpanDocument,
    canonical_text: String,
}

#[derive(Serialize)]
struct OutlineDocument {
    stable_key: String,
    parent_key: Option<String>,
    kind: &'static str,
    name: String,
    span: SpanDocument,
    signature: String,
    retained_in_mode: bool,
}

#[derive(Serialize)]
struct SpanDocument {
    start_line: usize,
    end_line: usize,
    start_byte: usize,
    end_byte: usize,
}

fn file_document(file: &FileOutline) -> FileDocument {
    FileDocument {
        path: file.path().to_string(),
        language: language_name(file.language()),
        projection: projection_document(&file.projection),
        outline: file
            .outline
            .iter()
            .map(|item| outline_document(file, item))
            .collect(),
    }
}

fn projection_document(file: &ProjectedFile) -> ProjectionDocument {
    ProjectionDocument {
        text: file.canonical_text().to_owned(),
        items: file.items().iter().map(item_document).collect(),
    }
}

fn item_document(item: &ProjectedItem) -> ItemDocument {
    ItemDocument {
        stable_key: item.stable_key.clone(),
        parent_key: item.parent_key.clone(),
        kind: kind_name(item.kind),
        name: item.name.clone(),
        span: span_document(&item.span),
        canonical_text: item.canonical_text.clone(),
    }
}

fn outline_document(file: &FileOutline, item: &OutlineItem) -> OutlineDocument {
    OutlineDocument {
        stable_key: item.item.stable_key.clone(),
        parent_key: item.item.parent_key.clone(),
        kind: kind_name(item.item.kind),
        name: item.item.name.clone(),
        span: span_document(&item.item.span),
        signature: item.item.canonical_text.clone(),
        retained_in_mode: file.retained_in_mode(item),
    }
}

/// Converts an adapter span to the wire form: one-based lines, zero-based bytes.
fn span_document(span: &SourceSpan) -> SpanDocument {
    SpanDocument {
        start_line: span.start_line() + 1,
        end_line: span.end_line() + 1,
        start_byte: span.start_byte(),
        end_byte: span.end_byte(),
    }
}

fn mode_name(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
    }
}

fn language_name(language: Language) -> &'static str {
    match language {
        Language::Elm => "elm",
        Language::Haskell => "haskell",
        Language::Python => "python",
        Language::Rust => "rust",
    }
}

/// Maps every [`ItemKind`] to its wire name. There is intentionally no wildcard
/// arm, so adding a kind is a compile error rather than a silent fallback.
fn kind_name(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Module => "module",
        ItemKind::Type => "type",
        ItemKind::TypeAlias => "type_alias",
        ItemKind::Constructor => "constructor",
        ItemKind::Field => "field",
        ItemKind::Variant => "variant",
        ItemKind::Trait => "trait",
        ItemKind::TraitImplementation => "trait_implementation",
        ItemKind::AssociatedType => "associated_type",
        ItemKind::TypeFamily => "type_family",
        ItemKind::PatternSynonym => "pattern_synonym",
        ItemKind::Function => "function",
        ItemKind::Method => "method",
        ItemKind::Value => "value",
        ItemKind::Constant => "constant",
        ItemKind::Static => "static",
        ItemKind::Port => "port",
        ItemKind::Operator => "operator",
        ItemKind::ForeignBlock => "foreign_block",
    }
}
