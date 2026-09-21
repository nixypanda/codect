//! The application chrome: a header with identity and context, and a footer
//! with contextual information and key hints.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{Content, LoadRequest, Model, Overlay, Pane, mode_label};
use crate::theme::{Rgb, Theme};

use super::text::clip_line;

/// Renders the header: brand, repository, selection, and context chips.
pub(crate) fn render_header(model: &Model, frame: &mut Frame, area: Rect) {
    let theme = &model.theme;
    let base = theme.bg(theme.palette.surface);
    let mut spans = vec![
        Span::styled(
            " ◆ ownai ",
            theme
                .fg_bg(theme.palette.bg, theme.palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", clip_tail(&model.root, 40)),
            theme.fg(theme.palette.text_dim),
        ),
    ];

    if let Some(path) = &model.selected {
        spans.push(Span::styled("› ".to_owned(), theme.fg(theme.palette.text_muted)));
        spans.push(Span::styled(
            path.to_string(),
            theme.fg(theme.palette.text),
        ));
    }

    let mut right = vec![
        chip(mode_label(model.mode), theme.palette.accent, theme),
        Span::raw(" "),
    ];
    match &model.request {
        LoadRequest::Show { revision, .. } => {
            right.push(chip(revision, theme.palette.surface_alt, theme));
        }
        LoadRequest::Diff { base, target, .. } => {
            right.push(chip(
                &format!("{base}..{target}"),
                theme.palette.surface_alt,
                theme,
            ));
        }
    }
    right.push(Span::raw(" "));
    right.push(chip(
        &format!("scope: {}", model.scope_label),
        theme.palette.surface_alt,
        theme,
    ));

    let left_width: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let right_width: usize = right
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let area_width = area.width as usize;
    if left_width + right_width + 1 <= area_width {
        spans.push(Span::styled(
            " ".repeat(area_width - left_width - right_width),
            base,
        ));
    } else {
        // Not enough room for both; drop the chips and keep the identity.
        right.clear();
    }
    spans.extend(right);

    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

/// Renders the footer: status on the left, contextual hints on the right.
pub(crate) fn render_status(model: &Model, frame: &mut Frame, area: Rect) {
    let theme = &model.theme;
    let base = theme.bg(theme.palette.surface);

    let mut left: Vec<Span<'static>> = Vec::new();
    if let Some(path) = &model.selected {
        left.push(Span::styled(
            format!(" {} ", path),
            theme.fg(theme.palette.text),
        ));
    } else {
        left.push(Span::styled(
            " no selection ".to_owned(),
            theme.fg(theme.palette.text_muted),
        ));
    }

    if let Some((added, removed)) = diff_stats(model) {
        left.push(Span::styled(
            format!("+{added} "),
            theme.fg(theme.palette.add_fg),
        ));
        left.push(Span::styled(
            format!("−{removed} "),
            theme.fg(theme.palette.del_fg),
        ));
    } else if let Some(text) = model.active_text() {
        left.push(Span::styled(
            format!("{} lines ", text.lines().count()),
            theme.fg(theme.palette.text_muted),
        ));
    }

    let hints = hints(model);
    let hints_style = theme.fg(theme.palette.text_muted);
    let left_width: usize = left
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let hints_width = UnicodeWidthStr::width(hints.as_str());
    let area_width = area.width as usize;

    let mut spans = left;
    if left_width + hints_width + 2 <= area_width {
        spans.push(Span::styled(
            " ".repeat(area_width - left_width - hints_width),
            base,
        ));
        spans.push(Span::styled(hints, hints_style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

/// The key hints for the current focus, content, and overlay.
fn hints(model: &Model) -> String {
    match &model.overlay {
        Some(Overlay::Revision { .. }) => "Enter apply   Esc cancel ".to_owned(),
        Some(Overlay::Scope(_)) => "↑↓ choose   Enter select   Esc close ".to_owned(),
        Some(Overlay::Mode { .. }) => "↑↓ choose   Enter apply   Esc close ".to_owned(),
        Some(Overlay::Help) => "Esc close ".to_owned(),
        None => match (model.focus, &model.content) {
            (Pane::Tree, _) => "↵ open   Tab content   m mode   s scope   ? help ".to_owned(),
            (Pane::Body, _) => "j/k scroll   h/l pan   Tab tree   ? help ".to_owned(),
            (Pane::Diff, _) => "j/k scroll   Tab tree   ? help ".to_owned(),
        },
    }
}

/// Added and removed logical-line counts for the selected diff.
pub(crate) fn diff_stats(model: &Model) -> Option<(usize, usize)> {
    if !matches!(model.content, Content::Diff(_)) {
        return None;
    }
    let mut added = 0;
    let mut removed = 0;
    for row in model.diff_rows() {
        if row.continuation {
            continue;
        }
        match row.kind {
            crate::app::VisualRowKind::Diff(ownai_core::DiffRowKind::Add) => added += 1,
            crate::app::VisualRowKind::Diff(ownai_core::DiffRowKind::Delete) => removed += 1,
            crate::app::VisualRowKind::Diff(ownai_core::DiffRowKind::Change) => {
                added += 1;
                removed += 1;
            }
            crate::app::VisualRowKind::Diff(ownai_core::DiffRowKind::Equal)
            | crate::app::VisualRowKind::Hunk => {}
        }
    }
    Some((added, removed))
}

/// Renders a transient diagnostic as a dismissible toast above the footer.
pub(crate) fn render_diagnostic(model: &Model, frame: &mut Frame, area: Rect) {
    let Some(text) = &model.diagnostic else {
        return;
    };
    let width = area.width.saturating_sub(4).min(90);
    if width == 0 || area.height < 2 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height - 2,
        width,
        height: 1,
    };
    let theme = &model.theme;
    frame.render_widget(
        Paragraph::new(clip_line(text, 0, width as usize)).style(
            theme
                .fg_bg(theme.palette.bg, theme.palette.danger)
                .add_modifier(Modifier::BOLD),
        ),
        popup,
    );
}

pub(crate) fn render_too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(
        Paragraph::new("terminal too small").style(theme.fg(theme.palette.text_muted)),
        area,
    );
}

fn chip(label: &str, background: Rgb, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        theme.fg_bg(theme.palette.bg, background),
    )
}

/// Keeps the tail of a path so the deepest directory stays visible.
fn clip_tail(text: &str, width: usize) -> String {
    let text_width = UnicodeWidthStr::width(text);
    if text_width <= width {
        return text.to_owned();
    }
    let keep = width.saturating_sub(1);
    let start = text.len().saturating_sub(keep);
    let start = (0..=start)
        .rev()
        .find(|&index| text.is_char_boundary(index))
        .unwrap_or(0);
    format!("…{}", &text[start..])
}
