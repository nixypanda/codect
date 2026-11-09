// The `codect.show.v1` and `codect.diff.v2` JSON documents.
//
// The JSON types live in the CLI so `base` stays free of serialization
// and `engine` stays free of file formats. The documents are produced for
// both committed revisions and editor-supplied bytes (stdin or worktree), and
// are validated against the schemas in `docs/schema/` by the test suite.
//
// # Contract highlights
//
// - `schema` is the literal `codect.show.v1` or `codect.diff.v2`; a consumer
//   treats any other value as a fatal, explicit version mismatch. Show stays on
//   v1; diff moved to v2 when file moves added `status: "moved"` and
//   `old_path`.
// - New show documents carry the additive `path_encoding: "codect.path.v1"`
//   marker: path fields then use Codect's injective display form. Legacy show
//   v1 documents omit it and their paths must be decoded with the old rules or
//   rejected when that cannot be done safely. Diff v2 always uses the injective
//   form and needs no marker.
// - Line numbers are **one-based** for editor friendliness. Byte offsets are
//   zero-based into the decoded UTF-8 source. The plugin converts as needed.
// - `projection` is the requested mode; `outline` is mode-independent and
//   complete, derived from the Signatures superset projection. For a
//   declaration the requested mode retains, the mode-correct closed-fold text
//   is the matching `projection.items[].canonical_text`; `outline.signature`
//   is only the fallback for `retained_in_mode: false` and is intentionally
//   the superset form. Container declarations (trait/impl/module) therefore
//   differ between the two.
// - `span` starts at the declaration node and excludes preceding attributes,
//   decorators, `{-# ... #-}` pragmas, and doc comments, which `signature`
//   may include. An editor extends a fold start upward over those lines; a
//   `decorator_start_line` field can be added additively within v1 later.
// - `stable_key` is unique within its file, not across the repository: Rust
//   `impl` keys are not path-namespaced, so consumers key global state by
//   `(path, stable_key)`.
// - JSON output contains no ANSI and is unaffected by `--color`.
//
// Documents are pretty-printed and always end with a single newline.

use base::{
    FileOutline, FileOutlineDiff, ItemKind, Language, OutlineItem, ProjectedFile, ProjectedItem,
    ProjectionMode, SourceSpan,
};
use engine::SnapshotDiff;
use serde::Serialize;

pub const SCHEMA: &str = "codect.show.v1";
pub const PATH_ENCODING: &str = "codect.path.v1";
pub const DIFF_SCHEMA: &str = "codect.diff.v2";

/// A diff document cannot be serialized without violating its contract.
///
/// This is an internal producer failure rather than user input: core owns the
/// rule that a move has two distinct raw paths, and the RepoPath display form
/// is injective. The CLI still returns an error instead of panicking or
/// emitting a malformed document if either invariant regresses.
#[derive(Debug, thiserror::Error)]
pub enum DiffDocumentError {
    #[error("a moved file has indistinguishable raw or escaped old and new paths")]
    IndistinguishableMovePaths,
}

pub fn diff_document(
    base: &str,
    target: &str,
    mode: ProjectionMode,
    diff: &SnapshotDiff,
    merge_base: bool,
) -> Result<String, DiffDocumentError> {
    let files = diff
        .files
        .iter()
        .map(|file| diff_file_document(file, diff))
        .collect::<Result<Vec<_>, _>>()?;
    let document = DiffDocument {
        schema: DIFF_SCHEMA,
        mode: mode_name(mode),
        base: SnapshotDocument {
            kind: diff.base_kind,
            revision: base,
            id: diff.base_id.clone(),
            merge_base: merge_base.then_some(true),
        },
        target: SnapshotDocument {
            kind: diff.target_kind,
            revision: target,
            id: diff.target_id.clone(),
            merge_base: None,
        },
        files,
    };
    let mut text =
        serde_json::to_string_pretty(&document).expect("serializing the diff document cannot fail");
    text.push('\n');
    Ok(text)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_base: Option<bool>,
}

#[derive(Serialize)]
struct DiffFileDocument {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
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

fn diff_file_document(
    file: &FileOutlineDiff,
    diff: &SnapshotDiff,
) -> Result<DiffFileDocument, DiffDocumentError> {
    match file {
        FileOutlineDiff::Added { new } => Ok(DiffFileDocument {
            path: new.path().to_string(),
            old_path: None,
            language: language_name(new.language()),
            status: "added",
            base: None,
            target: Some(diff_side_document(new, diff.target_id.clone())),
            equal: false,
        }),
        FileOutlineDiff::Deleted { old } => Ok(DiffFileDocument {
            path: old.path().to_string(),
            old_path: None,
            language: language_name(old.language()),
            status: "deleted",
            base: Some(diff_side_document(old, diff.base_id.clone())),
            target: None,
            equal: false,
        }),
        FileOutlineDiff::Modified { old, new } => Ok(DiffFileDocument {
            path: new.path().to_string(),
            old_path: None,
            language: language_name(new.language()),
            status: "modified",
            base: Some(diff_side_document(old, diff.base_id.clone())),
            target: Some(diff_side_document(new, diff.target_id.clone())),
            equal: false,
        }),
        FileOutlineDiff::Moved { old, new } => {
            // JSON Schema cannot compare `old_path` with `path`. Compare both
            // the raw paths and their serialized display forms so a regression
            // in either core invariant becomes a normal fatal diagnostic rather
            // than a schema-invalid document.
            let old_path = old.path().to_string();
            let new_path = new.path().to_string();
            if old.path() == new.path() || old_path == new_path {
                return Err(DiffDocumentError::IndistinguishableMovePaths);
            }
            let equal = old.projection.canonical_text() == new.projection.canonical_text();
            Ok(DiffFileDocument {
                path: new_path,
                old_path: Some(old_path),
                language: language_name(new.language()),
                status: "moved",
                base: Some(diff_side_document(old, diff.base_id.clone())),
                target: Some(diff_side_document(new, diff.target_id.clone())),
                equal,
            })
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Input {
    Revision,
    Stdin,
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

pub fn document(
    input: Input,
    revision: Option<&str>,
    mode: ProjectionMode,
    files: &[FileOutline],
) -> String {
    let document = Document {
        schema: SCHEMA,
        path_encoding: PATH_ENCODING,
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
    path_encoding: &'static str,
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
        ProjectionMode::Tests => "tests",
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn empty_outline(path: &str) -> FileOutline {
        let path = base::SupportedPath::new(base::RepoPath::new(path).expect("valid test path"))
            .expect("supported test path");
        FileOutline {
            projection: ProjectedFile::try_new(path, Vec::new()).expect("valid fixture"),
            retained: BTreeSet::new(),
            outline: Vec::new(),
        }
    }

    #[test]
    fn moved_json_producer_reports_indistinguishable_old_and_new_paths() {
        let diff = SnapshotDiff {
            base_kind: "commit",
            target_kind: "commit",
            base_id: "BASE".to_owned(),
            target_id: "TARGET".to_owned(),
            files: vec![FileOutlineDiff::Moved {
                old: empty_outline("src/same.rs"),
                new: empty_outline("src/same.rs"),
            }],
        };
        let error = diff_document("base", "target", ProjectionMode::Types, &diff, false)
            .expect_err("identical raw move paths must not serialize");
        assert!(matches!(
            error,
            DiffDocumentError::IndistinguishableMovePaths
        ));
    }
}
