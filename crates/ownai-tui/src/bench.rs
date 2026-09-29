//! Benchmark-only façade for the terminal frontend.
//!
//! This module exists so `benches/frame.rs` can reach the crate's internals
//! (`Model`, `view`, the syntax highlighter, and the diff-layout cache) without
//! widening the frontend's real public API. It is compiled only with the `bench`
//! feature, `#[doc(hidden)]`, and is not part of any supported surface.
//!
//! Everything here is a thin wrapper: the model builders drive the same pure
//! `update` path production uses, and the seam functions call the exact
//! functions a frame calls. A benchmark measured through this module therefore
//! measures real code, not a copy.

#![doc(hidden)]

use ownai_core::{
    FileDiff, ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath,
    Selection, SourceSpan,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::app::Content;
use crate::highlight::StyledLine;
use crate::icons::{IconStyle, Icons};
use crate::theme::{Capability, Flavor, Theme};
use crate::view::view;

pub use crate::app::{Key, LoadRequest, Model, Msg, VisualRow, update};

/// A `TestBackend` terminal of the given size.
///
/// The real driver draws through the crossterm backend; `TestBackend` is the
/// same `Terminal::draw` path with an in-memory surface, so it measures frame
/// assembly and ratatui's buffer diff but not escape-sequence writing.
pub fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).expect("test terminal")
}

/// Draws one frame of `model`, exactly as the runtime does.
pub fn draw(model: &Model, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| view(model, frame))
        .expect("draw a frame");
}

/// Draws one frame and returns the resulting surface, for buffer-diff benches.
pub fn render_to_buffer(model: &Model, width: u16, height: u16) -> Buffer {
    let mut terminal = terminal(width, height);
    draw(model, &mut terminal);
    terminal.backend().buffer().clone()
}

/// The cells ratatui's `Terminal::flush` scans when it diffs two surfaces.
pub fn buffer_diff(previous: &Buffer, current: &Buffer) -> usize {
    previous.diff_iter(current).count()
}

/// Highlights `text` with the production highlighter and a fixed dark theme.
pub fn highlight(text: &str, language: Language) -> Vec<StyledLine> {
    crate::highlight::highlight(text, language, &truecolor_dark())
}

/// Recomputes the wrapped diff rows for the model, bypassing the cache.
pub fn compute_diff_rows(model: &Model) -> Vec<VisualRow> {
    model.compute_diff_rows()
}

/// Runs the selection-dependent work once, as the runtime does per batch.
pub fn settle(model: Model) -> Model {
    crate::app::settle(model)
}

/// A projected file whose canonical text is `text`, built the same way the
/// language adapters build one: a single top-level item with that fragment.
pub fn projected(path: &str, language: Language, text: &str) -> ProjectedFile {
    let path = RepoPath::new(path).expect("benchmark path is valid");
    let items = if text.is_empty() {
        Vec::new()
    } else {
        vec![ProjectedItem {
            stable_key: "item".to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: "item".to_owned(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: 0,
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
            },
            canonical_text: text.to_owned(),
        }]
    };
    ProjectedFile::new(path, language, items)
}

/// A one-path diff with the given old and new projections.
pub fn file_diff(
    path: &str,
    old: Option<(&str, Language)>,
    new: Option<(&str, Language)>,
) -> FileDiff {
    match (old, new) {
        (None, Some((text, language))) => FileDiff::Added {
            new: projected(path, language, text),
        },
        (Some((text, language)), None) => FileDiff::Deleted {
            old: projected(path, language, text),
        },
        (Some((old_text, old_language)), Some((new_text, new_language))) => FileDiff::Modified {
            old: projected(path, old_language, old_text),
            new: projected(path, new_language, new_text),
        },
        (None, None) => panic!("a benchmark diff needs at least one side"),
    }
}

/// A model showing `files`, installed and settled through the real runtime
/// paths so highlighting and the derived cache are populated as at runtime.
pub fn show_model(files: Vec<ProjectedFile>, width: u16, height: u16) -> Model {
    let request = LoadRequest::Show {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
    };
    settle(load_show(files, &base(request, width, height)))
}

/// A model diffing `diffs`, installed and settled through the real runtime
/// paths.
pub fn diff_model(diffs: Vec<FileDiff>, width: u16, height: u16) -> Model {
    let request = LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: crate::DiffView::Range,
    };
    settle(load_diff(diffs, &base(request, width, height)))
}

/// Applies a completed `show` projection, as the runtime does on `Msg::Loaded`.
pub fn load_show(files: Vec<ProjectedFile>, model: &Model) -> Model {
    let request = model.request.clone();
    apply(model, request, files.into())
}

/// Applies a completed `diff` projection, as the runtime does on `Msg::Loaded`.
pub fn load_diff(diffs: Vec<FileDiff>, model: &Model) -> Model {
    let request = model.request.clone();
    apply(model, request, diffs.into())
}

fn apply(model: &Model, request: LoadRequest, content: Content) -> Model {
    let (next, _) = update(
        Msg::Loaded {
            request,
            result: Ok(content),
        },
        model,
    );
    next
}

fn base(request: LoadRequest, width: u16, height: u16) -> Model {
    Model::new(
        "/repo".to_owned(),
        request,
        "all".to_owned(),
        width,
        height,
        truecolor_dark(),
        Icons::new(IconStyle::None),
    )
}

/// The dark palette at full truecolor, matching the runtime's default.
fn truecolor_dark() -> Theme {
    Theme::new(Flavor::Dark, Capability::TrueColor)
}
