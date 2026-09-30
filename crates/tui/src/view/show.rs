use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::app::App;
use crate::page::Loaded;
use crate::render::block::pane_block;
use crate::render::highlight;
use crate::render::layout::Edge;
use crate::render::text::clip_line;
use crate::render::theme::Theme;

use crate::render::empty::render_empty;

pub(crate) fn render_show_body(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let theme = &app.chrome.theme;
    let block = pane_block(" Projection ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Loaded::Show(show) = &app.loaded else {
        return;
    };
    let Some(text) = show.active_text() else {
        let (title, detail) = if app.is_busy() && app.tree.rows.is_empty() {
            ("Loading", "Projecting files…")
        } else if app.tree.rows.is_empty() {
            ("No files", "No projected file content in this scope.")
        } else {
            ("No file selected", "Select a file to view its projection.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    };

    let total = show.line_count();
    let height = inner.height as usize;
    let needs_scrollbar = total > height;
    let gutter = gutter_width(total);
    let width = (inner.width as usize).saturating_sub(gutter + usize::from(needs_scrollbar));
    let skip = show.body.scroll as usize;
    let hscroll = show.body.hscroll as usize;

    let mut lines = Vec::new();
    match show.active_lines() {
        Some(styled) => {
            for (offset, runs) in styled.iter().skip(skip).take(height).enumerate() {
                let number = skip + offset + 1;
                let ranges = show.search_ranges(number);
                let runs = highlight_search(runs, &ranges, theme);
                let clipped = highlight::clip_runs(&runs, hscroll, width);
                let mut spans = vec![gutter_span(number, gutter, theme)];
                spans.extend(
                    clipped
                        .into_iter()
                        .map(|run| Span::styled(run.text, run.style)),
                );
                lines.push(Line::from(spans));
            }
        }
        None => {
            for (offset, line) in text.lines().skip(skip).take(height).enumerate() {
                let number = skip + offset + 1;
                lines.push(Line::from(vec![
                    gutter_span(number, gutter, theme),
                    Span::styled(
                        clip_line(line, hscroll, width),
                        theme.fg(theme.palette.text),
                    ),
                ]));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if needs_scrollbar {
        let mut state = ScrollbarState::new(total)
            .position(show.body.scroll as usize)
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
    total.max(1).to_string().len() + 1
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

fn gutter_span(number: usize, gutter: usize, theme: &Theme) -> Span<'static> {
    let field = gutter.saturating_sub(1);
    Span::styled(format!("{number:>field$} "), theme.fg(theme.palette.gutter))
}
