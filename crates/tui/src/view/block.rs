// Panel chrome shared by every pane: the bordered block and the shared divider.
//
// This is the presentation half of the geometry in [`crate::layout`]: the
// layout decides *where* each pane sits, and this decides how its frame is
// styled and drawn.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};

use crate::layout::Edge;
use crate::theme::Theme;

fn borders_for(edge: Edge) -> Borders {
    match edge {
        Edge::Solo => Borders::ALL,
        Edge::Left => Borders::LEFT | Borders::TOP | Borders::BOTTOM,
        Edge::Middle => Borders::TOP | Borders::BOTTOM,
        Edge::Right => Borders::RIGHT | Borders::TOP | Borders::BOTTOM,
    }
}

pub(crate) fn pane_block(title: &str, focused: bool, theme: &Theme, edge: Edge) -> Block<'static> {
    let border = if focused {
        theme.fg(theme.palette.border_focus)
    } else {
        theme.fg(theme.palette.border)
    };
    Block::default()
        .borders(borders_for(edge))
        .border_type(BorderType::Rounded)
        .border_style(border)
        .padding(Padding::horizontal(1))
        .style(theme.bg(theme.palette.bg))
        .title(title.to_owned())
}

/// The bordered block every popup (overlay or prompt) is drawn on.
pub(crate) fn popup_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(theme.palette.border_focus))
        .style(theme.bg(theme.palette.surface))
        .title(Span::styled(
            format!(" {title} "),
            theme.fg(theme.palette.accent).add_modifier(Modifier::BOLD),
        ))
}

/// Dims everything behind a popup so the popup reads as a layer above it.
pub(crate) fn scrim(frame: &mut Frame, area: Rect) {
    frame
        .buffer_mut()
        .set_style(area, Style::default().add_modifier(Modifier::DIM));
}

pub(crate) fn render_divider(frame: &mut Frame, column: Rect, theme: &Theme) {
    if column.width == 0 || column.height == 0 {
        return;
    }
    let style = theme.fg(theme.palette.divider);
    let mut lines = Vec::with_capacity(column.height as usize);
    for row in 0..column.height {
        let symbol = match row {
            0 => "┬",
            row if row + 1 == column.height => "┴",
            _ => "│",
        };
        lines.push(ratatui::text::Line::from(Span::styled(
            symbol.to_owned(),
            style,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(theme.bg(theme.palette.bg)),
        column,
    );
}
