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
    Selection, SourceOutline, SourceSpan, SupportedPath, assemble_outline,
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

pub fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).expect("test terminal")
}

pub fn draw(model: &App, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| view(model, frame))
        .expect("draw a frame");
}

pub fn render_to_buffer(model: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = terminal(width, height);
    draw(model, &mut terminal);
    terminal.backend().buffer().clone()
}

pub fn buffer_diff(previous: &Buffer, current: &Buffer) -> usize {
    previous.diff_iter(current).count()
}

pub fn highlight(text: &str, language: Language) -> Vec<StyledLine> {
    crate::render::highlight::highlight(text, language, &truecolor_dark())
}

pub fn compute_diff_rows(model: &App) -> Vec<VisualRow> {
    model.compute_diff_rows()
}

pub fn settle(model: App) -> App {
    crate::app::settle(model)
}

pub fn set_body_scroll(model: &mut App, scroll: u16) {
    match &mut model.loaded {
        Loaded::Show(show) => show.body.scroll = scroll,
        Loaded::Diff(diff) => diff.set_body_scroll(scroll),
    }
}

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

pub fn source_outline(path: &str, source: &str) -> SourceOutline {
    source_outline_with(path, source, source)
}

pub fn source_outline_with(path: &str, source: &str, projection_text: &str) -> SourceOutline {
    let path = SupportedPath::new(RepoPath::new(path).expect("benchmark path is valid"))
        .expect("benchmark path has a supported extension");
    let span = whole_span(source);
    let items = |text: &str| -> Vec<ProjectedItem> {
        if text.is_empty() {
            Vec::new()
        } else {
            vec![ProjectedItem {
                stable_key: "item".to_owned(),
                parent_key: None,
                kind: ItemKind::Module,
                name: "item".to_owned(),
                span,
                canonical_text: text.to_owned(),
            }]
        }
    };
    let outline = assemble_outline(&path, items(projection_text), items(source))
        .expect("valid benchmark outline");
    SourceOutline {
        outline,
        source: source.to_owned(),
    }
}

fn whole_span(source: &str) -> SourceSpan {
    let end_line = source.matches('\n').count();
    let end_column = source
        .lines()
        .next_back()
        .map_or(0, |line| line.chars().count());
    SourceSpan::new(0, source.len(), 0, 0, end_line, end_column)
}

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

pub fn show_model(mode: ProjectionMode, files: Vec<SourceOutline>, width: u16, height: u16) -> App {
    let request = ShowRequest {
        revision: "HEAD".to_owned(),
        mode,
        selection: Selection::all(),
    };
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

pub fn diff_model(diffs: Vec<FileDiff>, width: u16, height: u16) -> App {
    let request = diff_request();
    let model = base(
        LoadRequest::Diff {
            base: request.base.clone(),
            target: request.target.clone(),
            mode: request.mode,
            selection: request.selection.clone(),
            view: request.view,
            merge_base: request.merge_base,
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

pub fn history_model(steps: Vec<CommitStep>, diffs: Vec<FileDiff>, width: u16, height: u16) -> App {
    let request = commits_request();
    let model = base(
        LoadRequest::Diff {
            base: request.base.clone(),
            target: request.target.clone(),
            mode: request.mode,
            selection: request.selection.clone(),
            view: request.view,
            merge_base: request.merge_base,
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

pub fn load_show(files: Vec<SourceOutline>, model: &App) -> App {
    let request = match &model.loaded {
        Loaded::Show(show) => ShowRequest {
            revision: show.revision.clone(),
            mode: model.workspace.query.mode,
            selection: model.workspace.query.scope.selection.clone(),
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

pub fn load_diff(diffs: Vec<FileDiff>, model: &App) -> App {
    let request = match &model.loaded {
        Loaded::Diff(diff) => DiffRequest {
            base: diff.base.clone(),
            target: diff.target.clone(),
            mode: model.workspace.query.mode,
            selection: model.workspace.query.scope.selection.clone(),
            view: match diff.view {
                DiffViewState::Range(_) => DiffView::Range,
                DiffViewState::Commits(_) => DiffView::Commits,
            },
            merge_base: diff.merge_base,
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
        merge_base: false,
    }
}

fn commits_request() -> DiffRequest {
    DiffRequest {
        base: "HEAD~3".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Commits,
        merge_base: false,
    }
}

fn truecolor_dark() -> Theme {
    Theme::new(Flavor::Dark, Capability::TrueColor)
}
