use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::content::{CommitsFocus, DiffViewState, Loaded};

use super::empty::render_empty;
use super::geom::{Edge, commit_offset, pane_block};
use super::text::truncate_ellipsis;

pub(crate) fn render_commits(app: &App, frame: &mut Frame, area: Rect, edge: Edge) {
    let Loaded::Diff(diff) = &app.loaded else {
        return;
    };
    let DiffViewState::Commits(commits) = &diff.view else {
        return;
    };
    let theme = &app.chrome.theme;
    let focused = commits.focus == CommitsFocus::Commits;
    let block = pane_block(" Commits ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let steps = commits.picker.steps_slice();
    if steps.is_empty() {
        let (title, detail) = if app.is_busy() {
            ("Loading", "Loading commits…")
        } else {
            ("No commits", "No commits in the selected range.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    let cursor = commits.picker.cursor();
    let offset = commit_offset(
        cursor,
        commits.picker.scroll(),
        steps.len(),
        inner.height as usize,
    );
    let mut lines = Vec::new();
    for (index, step) in steps
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
    {
        let selected = index == cursor;
        let style = if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
        let id = step.commit_id.to_string();
        let position = format!(" {}/{}", index + 1, steps.len());
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
