//! The source view's messages, key/mouse handling, and effect results.

use std::collections::BTreeMap;
use std::sync::Arc;

use base::{RepoPath, SourceOutline};
use engine::EngineError;
use ratatui::layout::Position;

use crate::kernel::cx::Cx;
use crate::kernel::nav::{Nav, page_step};
use crate::kernel::plugin::{Event, MouseInput, Out, QueryChange, Request};
use crate::kernel::search::{SearchMatch, SearchPane};
use crate::kernel::workspace::{FileSet, NO_FILES, Query, Scope};
use crate::util::input::{Key, MouseKind};

use super::ShowRequest;
use super::rows::{ShowBody, show_visible};
use super::{MOUSE_SCROLL_STEP, REVISION_PROMPT_TAG, SourceView};

/// The source view's effect results.
#[derive(Debug)]
pub enum Msg {
    ShowLoaded {
        request: ShowRequest,
        result: Result<Arc<[SourceOutline]>, Box<EngineError>>,
    },
}

/// The source view's effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    Load(ShowRequest),
}

pub(crate) fn body_key(view: &mut SourceView, key: Key, cx: &Cx) {
    let selected = cx.workspace.selected.as_ref();
    let max_h = view.max_line_width(selected).saturating_sub(1) as u16;
    match key {
        Key::Down | Key::Char('j') => {
            let row = view.cursor_row().map_or(0, |row| row + 1);
            view.move_cursor_to_row(row, cx.size.height);
        }
        Key::Up | Key::Char('k') => {
            let row = view.cursor_row().map_or(0, |row| row.saturating_sub(1));
            view.move_cursor_to_row(row, cx.size.height);
        }
        Key::Right | Key::Char('l') => {
            view.body.hscroll = view.body.hscroll.saturating_add(1).min(max_h);
        }
        Key::Left | Key::Char('h') => {
            view.body.hscroll = view.body.hscroll.saturating_sub(1);
        }
        _ => {}
    }
}

pub(crate) fn cursor_action(view: &mut SourceView, nav: Nav, cx: &Cx) {
    let step = page_step(cx.size.height) as usize;
    let current = view.cursor_row().unwrap_or(0);
    let last = view.line_count().saturating_sub(1);
    let row = match nav {
        Nav::Top => 0,
        Nav::Bottom => last,
        Nav::PageUp => current.saturating_sub(step),
        Nav::PageDown => (current + step).min(last),
    };
    view.move_cursor_to_row(row, cx.size.height);
}

pub(crate) fn mouse(view: &mut SourceView, event: MouseInput, cx: &Cx) {
    let inner = event.geometry.inner;
    let point = Position {
        x: event.column,
        y: event.row,
    };
    match event.kind {
        MouseKind::Click => {
            if inner.contains(point) {
                let row =
                    view.body.scroll as usize + usize::from(event.row.saturating_sub(inner.y));
                if row < view.line_count() {
                    let marker = event.column - inner.x < 2;
                    if marker {
                        view.toggle_marker_region(row, cx);
                    } else {
                        view.move_cursor_to_row(row, cx.size.height);
                    }
                }
            }
        }
        MouseKind::ScrollUp | MouseKind::ScrollDown => {
            let delta = if event.kind == MouseKind::ScrollUp {
                -MOUSE_SCROLL_STEP
            } else {
                MOUSE_SCROLL_STEP
            };
            scroll_body(view, delta);
        }
    }
}

fn scroll_body(view: &mut SourceView, delta: i32) {
    let max = view.line_count().saturating_sub(1) as u16;
    let scroll = view.body.scroll;
    let next = if delta < 0 {
        scroll.saturating_sub(delta.unsigned_abs() as u16)
    } else {
        scroll.saturating_add(delta as u16).min(max)
    };
    view.body.scroll = next;
}

pub(crate) fn on_event(view: &mut SourceView, event: &Event, cx: &Cx) -> Vec<Out<Cmd>> {
    match event {
        Event::SelectionChanged => {
            view.body = ShowBody {
                scroll: 0,
                hscroll: 0,
                cursor_line: 1,
            };
            let selected = cx.workspace.selected.clone();
            view.ensure_visible_rows(selected.as_ref());
            Vec::new()
        }
        Event::Resized => {
            let selected = cx.workspace.selected.clone();
            view.ensure_visible_rows(selected.as_ref());
            view.reanchor_cursor(selected.as_ref());
            view.clamp_view(cx.size.height, selected.as_ref());
            Vec::new()
        }
        Event::RevealMatch(matched) if matched.pane == SearchPane::Source => {
            let selected = cx.workspace.selected.clone();
            view.ensure_visible_rows(selected.as_ref());
            reveal_match(view, matched, selected.as_ref(), cx.size.height);
            Vec::new()
        }
        Event::Requery(change) => {
            view.loading = true;
            let request = match change {
                QueryChange::Mode(mode) => {
                    let mut request = view.request(cx.workspace.query.scope.selection.clone());
                    request.mode = *mode;
                    request
                }
                QueryChange::Scope(selection) => view.request(selection.clone()),
            };
            vec![Out::Effect(Cmd::Load(request))]
        }
        Event::PromptSubmitted { tag, value }
            if *tag == REVISION_PROMPT_TAG && !value.is_empty() =>
        {
            view.loading = true;
            let mut request = view.request(cx.workspace.query.scope.selection.clone());
            request.revision = value.clone();
            vec![Out::Effect(Cmd::Load(request))]
        }
        _ => Vec::new(),
    }
}

fn reveal_match(
    view: &mut SourceView,
    matched: &SearchMatch,
    selected: Option<&RepoPath>,
    height: u16,
) {
    view.reveal_source_line(selected, matched.line);
    view.move_cursor_to_source_line(matched.line, height);
}

pub(crate) fn loaded(
    view: &mut SourceView,
    request: ShowRequest,
    result: Result<Arc<[SourceOutline]>, Box<EngineError>>,
) -> Vec<Out<Cmd>> {
    match result {
        Ok(files) => {
            view.loading = false;
            view.revision = request.revision;
            view.loaded_mode = request.mode;
            view.files = files;
            view.highlight.clear();
            view.folds.clear();
            view.visible_rows = Default::default();
            view.body = ShowBody {
                scroll: 0,
                hscroll: 0,
                cursor_line: 1,
            };
            let paths = Arc::from(show_visible(&view.files));
            vec![
                Out::Kernel(Request::QueryLoaded(Query {
                    mode: request.mode,
                    scope: Scope::new(request.selection),
                })),
                Out::Kernel(Request::SetFiles(FileSet {
                    paths,
                    badges: Arc::new(BTreeMap::new()),
                    empty: NO_FILES,
                })),
                Out::Kernel(Request::ClearDiagnostic),
                Out::Kernel(Request::CancelPrefix),
            ]
        }
        Err(error) => {
            view.loading = false;
            vec![Out::Kernel(Request::Diagnose(error.to_string()))]
        }
    }
}
