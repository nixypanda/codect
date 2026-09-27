//! The file tree: guides, selection, diff badges, and a scrollbar.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::UnicodeWidthStr;

use crate::app::{ChangeKind, Content, Model, RowKind, TreeRow};
use crate::theme::Theme;

use super::empty::render_empty;
use super::geom::{Edge, pane_block, window_offset};
use super::text::truncate_ellipsis;

pub(crate) fn render_tree(model: &Model, frame: &mut Frame, area: Rect, focused: bool, edge: Edge) {
    let block = pane_block(" Files ", focused, &model.theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if model.rows.is_empty() {
        let (title, detail) = if model.pending.is_some() {
            ("Loading", "Projecting files…")
        } else {
            match model.content {
                Content::Show(_) => ("No files", "No projected file content in this scope."),
                Content::Diff(_) => ("No changes", "Body-only edits are omitted."),
            }
        };
        render_empty(frame, inner, title, detail, &model.theme);
        return;
    }

    let height = inner.height as usize;
    let needs_scrollbar = model.rows.len() > height;
    let width = (inner.width as usize).saturating_sub(usize::from(needs_scrollbar));
    let offset = window_offset(model.cursor, model.rows.len(), height);
    let theme = &model.theme;

    let mut lines = Vec::new();
    for (index, row) in model.rows.iter().enumerate().skip(offset).take(height) {
        lines.push(tree_line(
            model,
            row,
            index,
            index == model.cursor,
            focused,
            width,
            theme,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if needs_scrollbar {
        let mut state = ScrollbarState::new(model.rows.len())
            .position(model.cursor)
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

fn tree_line(
    model: &Model,
    row: &TreeRow,
    index: usize,
    selected: bool,
    focused: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let base = if selected {
        let background = if focused {
            theme.palette.selection_bg
        } else {
            theme.palette.surface_alt
        };
        theme.fg_bg(theme.palette.selection_fg, background)
    } else {
        Style::default()
    };

    let mut spans: Vec<Span<'static>> = Vec::new();

    // The selection bar reserves its column on every row so labels stay aligned.
    if selected {
        let (bar, color) = if focused {
            ("▌", theme.palette.selection_bar)
        } else {
            ("▏", theme.palette.text_muted)
        };
        spans.push(Span::styled(bar.to_owned(), base.fg(theme.color(color))));
    } else {
        spans.push(Span::styled(" ".to_owned(), base));
    }

    // Guides: one three-column cell per ancestor level, then a connector.
    let rows = &model.rows;
    for level in 0..row.depth {
        let text = if level + 1 == row.depth {
            if is_last_child(rows, index, level) {
                "└─ "
            } else {
                "├─ "
            }
        } else if is_last_child(rows, index, level) {
            "   "
        } else {
            "│  "
        };
        spans.push(Span::styled(text.to_owned(), theme.fg(theme.palette.guide)));
    }

    let (marker, label_color) = match &row.kind {
        RowKind::Directory { expanded: true, .. } => ("▾ ", theme.palette.dir),
        RowKind::Directory {
            expanded: false, ..
        } => ("▸ ", theme.palette.dir),
        RowKind::File { .. } => ("", theme.palette.file),
    };
    if !marker.is_empty() {
        spans.push(Span::styled(
            marker.to_owned(),
            theme.fg(label_color).add_modifier(Modifier::BOLD),
        ));
    }

    // An opt-in Nerd Font glyph sits between the marker and the label.
    let icon = match &row.kind {
        RowKind::Directory { .. } => model.icons.folder(),
        RowKind::File { path } => model.icons.file(path),
    };
    if !icon.is_empty() {
        let color = match &row.kind {
            RowKind::Directory { .. } => theme.palette.dir,
            RowKind::File { .. } => theme.palette.text_muted,
        };
        spans.push(Span::styled(format!("{icon} "), theme.fg(color)));
    }

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let badge = badge(model, row);
    let badge_width = badge.as_ref().map_or(0, |(label, _)| label.len() + 1);
    let label_width = width.saturating_sub(used + badge_width);
    let label = truncate_ellipsis(&row.label, label_width);
    spans.push(Span::styled(label, base.fg(theme.color(label_color))));

    if let Some((label, color)) = badge {
        spans.push(Span::styled(
            format!(" {label}"),
            base.fg(theme.color(color)),
        ));
    }

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), base));
    }
    // Keep the selection fill behind guides and icons as well as the label.
    for span in &mut spans {
        span.style = base.patch(span.style);
    }
    Line::from(spans)
}

/// The diff badge for a file row, if any.
fn badge(model: &Model, row: &TreeRow) -> Option<(&'static str, crate::theme::Rgb)> {
    let RowKind::File { path } = &row.kind else {
        return None;
    };
    let theme = &model.theme;
    match model.change_kind(path)? {
        ChangeKind::Added => Some(("A", theme.palette.badge_add)),
        ChangeKind::Modified => Some(("M", theme.palette.badge_mod)),
        ChangeKind::Deleted => Some(("D", theme.palette.badge_del)),
    }
}

/// Whether the row at `index` is the last visible sibling at `level`.
fn is_last_child(rows: &[TreeRow], index: usize, level: usize) -> bool {
    for row in &rows[index + 1..] {
        if row.depth <= level {
            return row.depth < level;
        }
    }
    true
}
