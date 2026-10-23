//! The tree pane's rendering.
//!
//! Pure: it reads the tree state and the kernel's shared state through [`Cx`]
//! and draws into the frame. The selection comes from [`crate::workspace`], the
//! badges from the file set, and the empty state from the file set or the busy
//! flag.

use base::RepoPath;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::UnicodeWidthStr;

use crate::workspace::BadgeTone;
use tui_core::cx::Cx;
use tui_core::layout::Edge;
use tui_core::render::block::pane_block;
use tui_core::render::empty::render_empty;
use tui_core::render::text::truncate_ellipsis;
use tui_core::render::theme::{Rgb, Theme};

use super::{RowKind, Tree, TreeRow};

pub(crate) fn render(
    state: &Tree,
    cx: &Cx,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let theme = cx.theme;
    let block = pane_block(" Files ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.rows.is_empty() {
        let (title, detail) = if cx.busy {
            ("Loading", "Projecting files…")
        } else {
            let empty = cx.workspace.files.empty;
            (empty.title, empty.detail)
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    let height = inner.height as usize;
    let needs_scrollbar = state.rows.len() > height;
    let width = (inner.width as usize).saturating_sub(usize::from(needs_scrollbar));
    let offset = state.scroll.min(state.rows.len().saturating_sub(height));
    let selected = cx.workspace.selected.as_ref();

    let mut lines = Vec::new();
    for (index, row) in state.rows.iter().enumerate().skip(offset).take(height) {
        lines.push(tree_line(
            state,
            cx,
            row,
            index,
            index == state.cursor,
            focused,
            width,
            theme,
            selected,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if needs_scrollbar {
        let mut scrollbar = ScrollbarState::new(state.rows.len())
            .position(offset)
            .viewport_content_length(height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(theme.fg(theme.palette.border))
                .begin_symbol(None)
                .end_symbol(None),
            inner,
            &mut scrollbar,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn tree_line(
    state: &Tree,
    cx: &Cx,
    row: &TreeRow,
    index: usize,
    cursor: bool,
    focused: bool,
    width: usize,
    theme: &Theme,
    selected: Option<&RepoPath>,
) -> Line<'static> {
    let is_selected = matches!(&row.kind, RowKind::File { path } if Some(path) == selected);
    let highlighted = cursor && focused;
    let base = if is_selected || highlighted {
        let background = if highlighted {
            theme.palette.selection_bg
        } else {
            theme.palette.surface_alt
        };
        theme.fg_bg(theme.palette.selection_fg, background)
    } else {
        Style::default()
    };

    let mut spans: Vec<Span<'static>> = Vec::new();

    if highlighted {
        spans.push(Span::styled(
            "▌".to_owned(),
            base.fg(theme.color(theme.palette.selection_bar)),
        ));
    } else if is_selected {
        spans.push(Span::styled(
            "▏".to_owned(),
            base.fg(theme.color(theme.palette.text_muted)),
        ));
    } else {
        spans.push(Span::styled(" ".to_owned(), base));
    }

    let rows = &state.rows;
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

    let icon = match &row.kind {
        RowKind::Directory { .. } => cx.icons.folder(),
        RowKind::File { path } => cx.icons.file(path),
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
    let badge = badge(cx, row, theme);
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
    for span in &mut spans {
        span.style = base.patch(span.style);
    }
    Line::from(spans)
}

fn badge(cx: &Cx, row: &TreeRow, theme: &Theme) -> Option<(&'static str, Rgb)> {
    let RowKind::File { path } = &row.kind else {
        return None;
    };
    match cx.workspace.files.badges.get(path)? {
        BadgeTone::Added => Some(("A", theme.palette.badge_add)),
        BadgeTone::Modified => Some(("M", theme.palette.badge_mod)),
        BadgeTone::Deleted => Some(("D", theme.palette.badge_del)),
    }
}

fn is_last_child(rows: &[TreeRow], index: usize, level: usize) -> bool {
    for row in &rows[index + 1..] {
        if row.depth <= level {
            return row.depth < level;
        }
    }
    true
}
