//! Benchmark and docs-showcase façade for the terminal frontend.
//!
//! This module exists so `benches/frame.rs` (and `examples/frames.rs`, which
//! renders the docs/showcase images) can reach the crate's internals (`Model`,
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

use crate::kernel::app::{App, Msg as KernelMsg, settle as kernel_settle, update as kernel_update};
use crate::page::{DiffViewState, Loaded};
use crate::plugins::legacy::Msg as LegacyMsg;
use crate::plugins::{KernelOptions, Plugins, PluginsMsg, build};
use crate::render::highlight::StyledLine;
use crate::render::icons::{IconStyle, Icons};
use crate::render::layout::VisualRow;
use crate::render::theme::{Capability, Flavor, Theme};
use crate::route::{DiffRequest, DiffView, LoadRequest, ShowRequest};

pub type Model = App<Plugins>;
pub type Msg = KernelMsg<PluginsMsg>;

pub use crate::util::input::Key;

pub fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).expect("test terminal")
}

pub fn draw(model: &Model, terminal: &mut Terminal<TestBackend>) {
    terminal
        .draw(|frame| crate::kernel::view::view(model, frame))
        .expect("draw a frame");
}

pub fn render_to_buffer(model: &Model, width: u16, height: u16) -> Buffer {
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

pub fn compute_diff_rows(model: &Model) -> Vec<VisualRow> {
    match &model.plugins.legacy.loaded {
        Loaded::Diff(diff) => diff.compute_rows(
            model.kernel.size.width,
            model.kernel.size.height,
            model.kernel.chrome.tree_percent,
            &model.kernel.chrome.theme,
            model.kernel.workspace.selected.as_ref(),
        ),
        Loaded::Show(_) => Vec::new(),
    }
}

pub fn update(msg: Msg, model: &Model) -> (Model, Vec<crate::plugins::PluginsCmd>) {
    kernel_update(msg, model)
}

pub fn settle(model: Model) -> Model {
    kernel_settle(model)
}

pub fn set_body_scroll(model: &mut Model, scroll: u16) {
    match &mut model.plugins.legacy.loaded {
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

fn base(request: LoadRequest, width: u16, height: u16) -> Model {
    build(KernelOptions {
        root: "/repo".to_owned(),
        request,
        scope_label: "all".to_owned(),
        width,
        height,
        theme: truecolor_dark(),
        icons: Icons::new(IconStyle::None),
    })
    .0
}

pub fn show_model(
    mode: ProjectionMode,
    files: Vec<SourceOutline>,
    width: u16,
    height: u16,
) -> Model {
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
        Msg::Plugin(PluginsMsg::Legacy(LegacyMsg::ShowLoaded {
            request,
            result: Ok(files.into()),
        })),
    ))
}

pub fn diff_model(diffs: Vec<FileDiff>, width: u16, height: u16) -> Model {
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
        Msg::Plugin(PluginsMsg::Legacy(LegacyMsg::DiffLoaded {
            request,
            result: Ok(diffs.into()),
        })),
    ))
}

pub fn history_model(
    steps: Vec<CommitStep>,
    diffs: Vec<FileDiff>,
    width: u16,
    height: u16,
) -> Model {
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
        Msg::Plugin(PluginsMsg::Legacy(LegacyMsg::HistoryLoaded {
            request,
            result: Ok((steps.into(), diffs.into())),
        })),
    ))
}

pub fn load_show(files: Vec<SourceOutline>, model: &Model) -> Model {
    let request = match &model.plugins.legacy.loaded {
        Loaded::Show(show) => ShowRequest {
            revision: show.revision.clone(),
            mode: model.kernel.workspace.query.mode,
            selection: model.kernel.workspace.query.scope.selection.clone(),
        },
        Loaded::Diff(_) => show_request(),
    };
    apply(
        model.clone(),
        Msg::Plugin(PluginsMsg::Legacy(LegacyMsg::ShowLoaded {
            request,
            result: Ok(files.into()),
        })),
    )
}

pub fn load_diff(diffs: Vec<FileDiff>, model: &Model) -> Model {
    let request = match &model.plugins.legacy.loaded {
        Loaded::Diff(diff) => DiffRequest {
            base: diff.base.clone(),
            target: diff.target.clone(),
            mode: model.kernel.workspace.query.mode,
            selection: model.kernel.workspace.query.scope.selection.clone(),
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
        Msg::Plugin(PluginsMsg::Legacy(LegacyMsg::DiffLoaded {
            request,
            result: Ok(diffs.into()),
        })),
    )
}

fn apply(model: Model, msg: Msg) -> Model {
    kernel_update(msg, &model).0
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
