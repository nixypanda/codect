//! Pure geometry and panel chrome shared by the view and the derived-layout
//! cache. Nothing here reads the terminal or the model's behaviour.

use ownai_engine::FileDiff;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};

use crate::theme::Theme;

/// Splits a terminal of `width` × `height` into content and status. Pure, so
/// `update` and `view` agree.
pub(crate) fn frame_chunks(width: u16, height: u16) -> (Rect, Rect) {
    let area = Rect::new(0, 0, width, height);
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    (chunks[0], chunks[1])
}

/// Where a column sits among its neighbours, which decides which borders it
/// draws so adjacent panes share a single divider instead of doubling it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Edge {
    /// The only column: all four borders.
    Solo,
    /// The leftmost of several: no right border.
    Left,
    /// Between two dividers: only top and bottom borders.
    Middle,
    /// The rightmost of several: no left border.
    Right,
}

fn borders_for(edge: Edge) -> Borders {
    match edge {
        Edge::Solo => Borders::ALL,
        Edge::Left => Borders::LEFT | Borders::TOP | Borders::BOTTOM,
        Edge::Middle => Borders::TOP | Borders::BOTTOM,
        Edge::Right => Borders::RIGHT | Borders::TOP | Borders::BOTTOM,
    }
}

/// A rounded panel with a themed border. Focused panels use the accent border.
pub(crate) fn pane_block(
    title: &str,
    focused: bool,
    theme: &Theme,
    edge: Edge,
) -> Block<'static> {
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
        .title(title.to_owned())
}

/// Splits `area` into content columns separated by single-column dividers.
///
/// `weights` are relative widths. Returns `(columns, dividers)`, where
/// `dividers` has one fewer entry than `columns`. When there is not enough room
/// for every divider, they are dropped and the columns share the space.
pub(crate) fn split_with_dividers(area: Rect, weights: &[u16]) -> (Vec<Rect>, Vec<Rect>) {
    let count = weights.len();
    if count == 0 {
        return (Vec::new(), Vec::new());
    }
    if count == 1 {
        return (vec![area], Vec::new());
    }

    let total_weight: u32 = weights.iter().map(|&w| u32::from(w.max(1))).sum();
    let divider_count = count - 1;
    // Drop dividers if the area is too narrow to keep every column non-empty.
    let dividers = if area.width as usize > divider_count + count {
        divider_count
    } else {
        0
    };
    let available = u32::from(area.width) - dividers as u32;

    let mut widths = Vec::with_capacity(count);
    let mut assigned = 0u32;
    for (index, &weight) in weights.iter().enumerate() {
        let width = if index + 1 == count {
            available - assigned
        } else {
            available * u32::from(weight.max(1)) / total_weight
        };
        widths.push(width as u16);
        assigned += width;
    }

    let mut columns = Vec::with_capacity(count);
    let mut divider_rects = Vec::with_capacity(dividers);
    let mut x = area.x;
    for (index, width) in widths.iter().enumerate() {
        columns.push(Rect {
            x,
            y: area.y,
            width: *width,
            height: area.height,
        });
        x = x.saturating_add(*width);
        if index < dividers {
            divider_rects.push(Rect {
                x,
                y: area.y,
                width: 1,
                height: area.height,
            });
            x = x.saturating_add(1);
        }
    }
    (columns, divider_rects)
}

/// Draws a vertical divider, joining the neighbouring top and bottom borders.
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
        lines.push(ratatui::text::Line::from(Span::styled(symbol.to_owned(), style)));
    }
    frame.render_widget(Paragraph::new(lines).style(Style::default()), column);
}

/// The width of the line-number gutter: a sign column, the digits, and a space.
pub(crate) fn gutter_width(diff: &FileDiff) -> usize {
    let lines = |file: &Option<ownai_core::ProjectedFile>| {
        file.as_ref()
            .map_or(0, |file| file.canonical_text().lines().count())
    };
    lines(&diff.old)
        .max(lines(&diff.new))
        .max(1)
        .to_string()
        .len()
        + 2
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

pub(crate) fn window_offset(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height || cursor < height {
        0
    } else {
        (cursor + 1).saturating_sub(height)
    }
}
