//! The source view's pane: a gutter, the source rows, and a scrollbar.

use base::ProjectionMode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::kernel::cx::Cx;
use crate::kernel::search::SearchPane;
use crate::render::block::pane_block;
use crate::render::empty::render_empty;
use crate::render::highlight;
use crate::render::layout::Edge;
use crate::render::theme::Theme;

use super::SourceView;
use super::rows::{VisibleRow, fold_suffix};

pub(crate) fn render(
    view: &SourceView,
    cx: &Cx,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let selected = cx.workspace.selected.as_ref();
    let theme = cx.theme;
    let block = pane_block(" Source ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if view.active_source(selected).is_none() {
        let tree_empty = cx.workspace.files.paths.is_empty();
        let (title, detail) = if cx.busy && tree_empty {
            ("Loading", "Projecting files…")
        } else if tree_empty {
            ("No files", "No source files in this scope.")
        } else {
            ("No file selected", "Select a file to view its source.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    if view.line_count() == 0 {
        let (title, detail) = if view.loaded_mode == ProjectionMode::Tests {
            ("No test declarations", "Nothing retained in this mode.")
        } else {
            ("Empty file", "This file has no source lines.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    let total = view.line_count();
    let source_total = view.source_line_count(selected);
    let height = inner.height as usize;
    let needs_scrollbar = total > height;
    let gutter = gutter_width(source_total);
    let width = (inner.width as usize).saturating_sub(gutter + usize::from(needs_scrollbar));
    let skip = view.body.scroll as usize;
    let hscroll = view.body.hscroll as usize;
    let cursor_row = if focused { view.cursor_row() } else { None };
    let cursor_bg = theme.color(theme.palette.surface_alt);

    let mut lines = Vec::new();
    for (index, row) in view
        .visible_rows()
        .iter()
        .enumerate()
        .skip(skip)
        .take(height)
    {
        let number = row.source_line();
        let ranges = cx.workspace.ranges_on_line(SearchPane::Source, number);
        let mut runs = view.row_runs(selected, number, theme);
        runs = highlight_search(&runs, &ranges, theme);
        if let VisibleRow::Fold {
            hidden, dropped, ..
        } = row
        {
            runs.push(highlight::Run {
                style: theme.fg(theme.palette.text_dim),
                text: fold_suffix(*hidden, *dropped, view.loaded_mode),
            });
        }
        let mut clipped = highlight::clip_runs(&runs, hscroll, width);
        let mut marker = gutter_marker_span(view.fold_marker(selected, row), theme);
        let mut number = gutter_number_span(number, gutter, theme);
        if cursor_row == Some(index) {
            for run in &mut clipped {
                if run.style.bg.is_none() {
                    run.style = run.style.bg(cursor_bg);
                }
            }
            if marker.style.bg.is_none() {
                marker.style = marker.style.bg(cursor_bg);
            }
            if number.style.bg.is_none() {
                number.style = number.style.bg(cursor_bg);
            }
        }
        let mut spans = vec![marker, number];
        spans.extend(
            clipped
                .into_iter()
                .map(|run| Span::styled(run.text, run.style)),
        );
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if needs_scrollbar {
        let mut state = ScrollbarState::new(total)
            .position(view.body.scroll as usize)
            .viewport_content_length(height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(theme.fg(theme.palette.border))
                .begin_symbol(None)
                .end_symbol(None),
            inner,
            &mut state,
        );
    }
}

fn gutter_width(total: usize) -> usize {
    total.max(1).to_string().len() + 3
}

fn highlight_search(
    runs: &[highlight::Run],
    ranges: &[(usize, usize, bool)],
    theme: &Theme,
) -> Vec<highlight::Run> {
    let mut styled = runs.to_vec();
    for (start, end, current) in ranges {
        let color = if *current {
            theme.color(theme.palette.match_current_bg)
        } else {
            theme.color(theme.palette.match_bg)
        };
        styled = highlight::apply_emphasis(&styled, &[(*start, *end)], color);
    }
    styled
}

fn gutter_marker_span(marker: Option<char>, theme: &Theme) -> Span<'static> {
    match marker {
        Some(marker) => Span::styled(format!("{marker} "), theme.fg(theme.palette.accent)),
        None => Span::styled("  ".to_owned(), theme.fg(theme.palette.gutter)),
    }
}

fn gutter_number_span(number: usize, gutter: usize, theme: &Theme) -> Span<'static> {
    let field = gutter.saturating_sub(3);
    Span::styled(format!("{number:>field$} "), theme.fg(theme.palette.gutter))
}
