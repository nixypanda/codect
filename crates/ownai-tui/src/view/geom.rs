//! Pure geometry and panel chrome shared by the view and the derived-layout
//! cache. Nothing here reads the terminal or the model's behaviour.

use ownai_engine::FileDiff;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, BorderType, Borders};

use crate::theme::Theme;

/// Splits a terminal of `width` × `height` into content and status. Pure, so
/// `update` and `view` agree.
pub(crate) fn frame_chunks(width: u16, height: u16) -> (Rect, Rect) {
    let area = Rect::new(0, 0, width, height);
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    (chunks[0], chunks[1])
}

pub(crate) fn split_show_columns(content: Rect, tree_percent: u16) -> (Rect, Rect) {
    let columns = Layout::horizontal([
        Constraint::Percentage(tree_percent),
        Constraint::Percentage(100 - tree_percent),
    ])
    .split(content);
    (columns[0], columns[1])
}

pub(crate) fn split_diff_columns(content: Rect, tree_percent: u16) -> (Rect, Rect, Rect) {
    let rest = 100 - tree_percent;
    let side = rest / 2;
    let columns = Layout::horizontal([
        Constraint::Percentage(tree_percent),
        Constraint::Percentage(side),
        Constraint::Percentage(rest - side),
    ])
    .split(content);
    (columns[0], columns[1], columns[2])
}

/// A rounded panel with a themed border. Focused panels use the accent border.
pub(crate) fn pane_block(title: &str, focused: bool, theme: &Theme) -> Block<'static> {
    let border = if focused {
        theme.fg(theme.palette.border_focus)
    } else {
        theme.fg(theme.palette.border)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(title.to_owned())
}

/// The width of the line-number gutter, including its trailing space.
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
        + 1
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
