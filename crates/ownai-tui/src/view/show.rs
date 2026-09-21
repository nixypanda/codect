//! The projection pane: syntax-highlighted lines with a line-number gutter.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::app::Model;
use crate::highlight;
use crate::theme::Theme;

use super::geom::{Edge, pane_block};
use super::text::clip_line;

pub(crate) fn render_show_body(
    model: &Model,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let title = model
        .selected
        .as_ref()
        .map_or_else(|| " Projection ".to_owned(), |path| format!(" {path} "));
    let block = pane_block(&title, focused, &model.theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(text) = model.active_text() else {
        frame.render_widget(
            Paragraph::new("no projected files")
                .style(model.theme.fg(model.theme.palette.text_muted)),
            inner,
        );
        return;
    };

    let total = model.line_count();
    let height = inner.height as usize;
    let needs_scrollbar = total > height;
    let gutter = gutter_width(total);
    let width = (inner.width as usize)
        .saturating_sub(gutter + usize::from(needs_scrollbar));
    let skip = model.body_scroll as usize;
    let hscroll = model.body_hscroll as usize;
    let theme = &model.theme;

    let mut lines = Vec::new();
    match model.active_show_lines() {
        Some(styled) => {
            for (offset, runs) in styled.iter().skip(skip).take(height).enumerate() {
                let number = skip + offset + 1;
                let clipped = highlight::clip_runs(runs, hscroll, width);
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
            .position(model.body_scroll as usize)
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

fn gutter_span(number: usize, gutter: usize, theme: &Theme) -> Span<'static> {
    let field = gutter.saturating_sub(1);
    Span::styled(
        format!("{number:>field$} "),
        theme.fg(theme.palette.gutter),
    )
}
