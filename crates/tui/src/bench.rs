//! Benchmark and docs-showcase façade for the terminal frontend.
//!
//! This module exists so `benches/frame.rs` (and `examples/frames.rs`, which
//! renders the docs/showcase images) can reach the crate's internals (`App`,
//! `view`, the syntax highlighter, and the diff-layout cache) without widening
//! the frontend's real public API. It is compiled only with the `bench`
//! feature, `#[doc(hidden)]`, and is not part of any supported surface.
//!
//! Everything here is a thin wrapper: the model builders drive the same pure
//! `update` path production uses, and the seam functions call the exact
//! functions a frame calls. A benchmark measured through this module therefore
//! measures real code, not a copy.

#![doc(hidden)]

use base::{
    FileDiff, ItemKind, Language, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath,
    Selection, SourceSpan, SupportedPath,
};
use engine::CommitStep;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::app::{App, DiffRequest, DiffView, LoadRequest, ShowRequest};
use crate::page::{DiffViewState, Loaded};
use crate::render::highlight::StyledLine;
use crate::render::icons::{IconStyle, Icons};
use crate::render::layout::VisualRow;
use crate::render::theme::{Capability, Flavor, Theme};
use crate::view::view;

pub use crate::app::App as Model;
pub use crate::app::Msg;
pub use crate::app::update;
pub use crate::util::input::Key;

/// A `TestBackend` terminal of the given size.
///
/// The real driver draws through the crossterm backend; `TestBackend` is the
/// same `Terminal::draw` path with an in-memory surface, so it measures frame
/// assembly and ratatui's buffer diff but not escape-sequence writing.
pub fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).expect("test terminal")
}

/// Draws one frame of `model`, exactly as the runtime does.
pub fn draw(model: &App, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| view(model, frame))
        .expect("draw a frame");
}

/// Draws one frame and returns the resulting surface, for buffer-diff benches.
pub fn render_to_buffer(model: &App, width: u16, height: u16) -> Buffer {
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
    crate::render::highlight::highlight(text, language, &truecolor_dark())
}

/// Recomputes the wrapped diff rows for the model, bypassing the cache.
pub fn compute_diff_rows(model: &App) -> Vec<VisualRow> {
    model.compute_diff_rows()
}

/// Runs the selection-dependent work once, as the runtime does per batch.
pub fn settle(model: App) -> App {
    crate::app::settle(model)
}

/// Sets the body scroll offset of the loaded diff or show.
pub fn set_body_scroll(model: &mut App, scroll: u16) {
    match &mut model.loaded {
        Loaded::Show(show) => show.body.scroll = scroll,
        Loaded::Diff(diff) => diff.set_body_scroll(scroll),
    }
}

/// A projected file whose canonical text is `text`, built the same way the
/// language adapters build one: a single top-level item with that fragment.
///
/// The language is derived from `path`, so a caller cannot pass a language that
/// disagrees with the extension.
pub fn projected(path: &str, text: &str) -> ProjectedFile {
    let path = SupportedPath::new(RepoPath::new(path).expect("benchmark path is valid"))
        .expect("benchmark path has a supported extension");
    let items = if text.is_empty() {
        Vec::new()
    } else {
        vec![ProjectedItem {
            stable_key: "item".to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: "item".to_owned(),
            span: SourceSpan::new(0, 0, 0, 0, 0, 0),
            canonical_text: text.to_owned(),
        }]
    };
    ProjectedFile::try_new(path, items).expect("valid benchmark projection")
}

/// A one-path diff with the given old and new projections.
pub fn file_diff(path: &str, old: Option<&str>, new: Option<&str>) -> FileDiff {
    match (old, new) {
        (None, Some(text)) => FileDiff::Added {
            new: projected(path, text),
        },
        (Some(text), None) => FileDiff::Deleted {
            old: projected(path, text),
        },
        (Some(old_text), Some(new_text)) => FileDiff::Modified {
            old: projected(path, old_text),
            new: projected(path, new_text),
        },
        (None, None) => panic!("a benchmark diff needs at least one side"),
    }
}

fn base(request: LoadRequest, width: u16, height: u16) -> App {
    App::new(
        "/repo".to_owned(),
        request,
        "all".to_owned(),
        width,
        height,
        truecolor_dark(),
        Icons::new(IconStyle::None),
    )
}

/// A model showing `files`, installed and settled through the real runtime
/// paths so highlighting and the derived cache are populated as at runtime.
pub fn show_model(files: Vec<ProjectedFile>, width: u16, height: u16) -> App {
    let request = show_request();
    let model = base(
        LoadRequest::Show {
            revision: request.revision.clone(),
            mode: request.mode,
            selection: request.selection.clone(),
        },
        width,
        height,
    );
    settle(apply(
        model,
        Msg::ShowLoaded {
            request,
            result: Ok(files.into()),
        },
    ))
}

/// A model diffing `diffs`, installed and settled through the real runtime
/// paths.
pub fn diff_model(diffs: Vec<FileDiff>, width: u16, height: u16) -> App {
    let request = diff_request();
    let model = base(
        LoadRequest::Diff {
            base: request.base.clone(),
            target: request.target.clone(),
            mode: request.mode,
            selection: request.selection.clone(),
            view: request.view,
        },
        width,
        height,
    );
    settle(apply(
        model,
        Msg::DiffLoaded {
            request,
            result: Ok(diffs.into()),
        },
    ))
}

/// A commits-view model showing `steps` and the selected step's `diffs`,
/// installed and settled the way the runtime installs `Msg::HistoryLoaded`.
pub fn history_model(steps: Vec<CommitStep>, diffs: Vec<FileDiff>, width: u16, height: u16) -> App {
    let request = commits_request();
    let model = base(
        LoadRequest::Diff {
            base: request.base.clone(),
            target: request.target.clone(),
            mode: request.mode,
            selection: request.selection.clone(),
            view: request.view,
        },
        width,
        height,
    );
    settle(apply(
        model,
        Msg::HistoryLoaded {
            request,
            result: Ok((steps.into(), diffs.into())),
        },
    ))
}

/// Applies a completed `show` projection, as the runtime does on
/// `Msg::ShowLoaded`.
pub fn load_show(files: Vec<ProjectedFile>, model: &App) -> App {
    let request = match &model.loaded {
        Loaded::Show(show) => ShowRequest {
            revision: show.revision.clone(),
            mode: show.mode,
            selection: show.scope.selection.clone(),
        },
        Loaded::Diff(_) => show_request(),
    };
    apply(
        model.clone(),
        Msg::ShowLoaded {
            request,
            result: Ok(files.into()),
        },
    )
}

/// Applies a completed `diff` projection, as the runtime does on
/// `Msg::DiffLoaded`.
pub fn load_diff(diffs: Vec<FileDiff>, model: &App) -> App {
    let request = match &model.loaded {
        Loaded::Diff(diff) => DiffRequest {
            base: diff.base.clone(),
            target: diff.target.clone(),
            mode: diff.mode,
            selection: diff.scope.selection.clone(),
            view: match diff.view {
                DiffViewState::Range(_) => DiffView::Range,
                DiffViewState::Commits(_) => DiffView::Commits,
            },
        },
        Loaded::Show(_) => diff_request(),
    };
    apply(
        model.clone(),
        Msg::DiffLoaded {
            request,
            result: Ok(diffs.into()),
        },
    )
}

fn apply(model: App, msg: Msg) -> App {
    crate::app::update(msg, &model).0
}

fn show_request() -> ShowRequest {
    ShowRequest {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
    }
}

fn diff_request() -> DiffRequest {
    DiffRequest {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Range,
    }
}

fn commits_request() -> DiffRequest {
    DiffRequest {
        base: "HEAD~3".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Commits,
    }
}

/// The dark palette at full truecolor, matching the runtime's default.
fn truecolor_dark() -> Theme {
    Theme::new(Flavor::Dark, Capability::TrueColor)
}
