//! Scrollable commit picker above the file tree.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::Model;

use super::empty::render_empty;
use super::geom::{Edge, commit_offset, pane_block};
use super::text::truncate_ellipsis;

pub(crate) fn render_commits(model: &Model, frame: &mut Frame, area: Rect, edge: Edge) {
    let block = pane_block(
        " Commits ",
        model.focus == crate::app::Pane::Commits,
        &model.theme,
        edge,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if model.commits.is_empty() {
        let (title, detail) = if model.pending.is_some() {
            ("Loading", "Loading commits…")
        } else {
            ("No commits", "No commits in the selected range.")
        };
        render_empty(frame, inner, title, detail, &model.theme);
        return;
    }

    let offset = commit_offset(
        model.commit_cursor,
        model.commit_scroll,
        model.commits.len(),
        inner.height as usize,
    );
    let mut lines = Vec::new();
    for (index, step) in model
        .commits
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
    {
        let selected = index == model.commit_cursor;
        let style = if selected {
            model.theme.fg_bg(
                model.theme.palette.selection_fg,
                model.theme.palette.selection_bg,
            )
        } else {
            model.theme.fg(model.theme.palette.text)
        };
        let id = step.commit_id.to_string();
        let position = format!(" {}/{}", index + 1, model.commits.len());
        let prefix = format!(
            "{} {} ",
            if selected { "▌" } else { " " },
            &id[..id.len().min(7)]
        );
        let used = prefix.len() + position.len();
        let subject = truncate_ellipsis(&step.subject, (inner.width as usize).saturating_sub(used));
        let mut row = format!("{prefix}{subject}");
        let padding = (inner.width as usize)
            .saturating_sub(unicode_width::UnicodeWidthStr::width(row.as_str()) + position.len());
        row.push_str(&" ".repeat(padding));
        row.push_str(&position);
        lines.push(Line::from(Span::styled(
            row,
            if selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            },
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
