use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{Content, LoadRequest, Model, Overlay, Pane, mode_label};
use crate::theme::{Rgb, Theme};

use super::text::clip_line;

pub(crate) fn render_header(model: &Model, frame: &mut Frame, area: Rect) {
    let theme = &model.theme;
    let base = theme.bg(theme.palette.surface);
    let brand = Span::styled(
        " ◆ ownai ",
        theme
            .fg_bg(theme.ink(theme.palette.accent), theme.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
    let revision = match &model.request {
        LoadRequest::Show { revision, .. } => revision.clone(),
        LoadRequest::Diff { .. } => {
            let (base, target) = model.diff_revisions().expect("diff request has revisions");
            format!("{base}..{target}")
        }
    };
    let context = [
        chip(mode_label(model.mode), theme.palette.accent, theme),
        chip(&revision, theme.palette.surface_alt, theme),
        chip(
            &format!("scope: {}", model.scope_label),
            theme.palette.surface_alt,
            theme,
        ),
    ];
    let area_width = area.width as usize;
    let brand_width = UnicodeWidthStr::width(brand.content.as_ref());
    let mut shown = context.len();
    let context_width = |count: usize| {
        context[..count]
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()) + 1)
            .sum::<usize>()
    };
    // Leave a useful piece of the repository visible before dropping scope,
    // then revisions, on narrow terminals.
    while shown > 0 && brand_width + context_width(shown) + 10 > area_width {
        shown -= 1;
    }
    let right_width = context_width(shown);
    let root_width = area_width
        .saturating_sub(brand_width + right_width + 2)
        .min(40);
    let mut spans = vec![
        brand,
        Span::styled(
            format!(" {} ", clip_tail(&model.root, root_width)),
            theme.fg(theme.palette.text_dim),
        ),
    ];
    let used = brand_width + UnicodeWidthStr::width(spans[1].content.as_ref());
    spans.push(Span::styled(
        " ".repeat(area_width.saturating_sub(used + right_width)),
        base,
    ));
    for span in context.into_iter().take(shown) {
        spans.push(Span::raw(" "));
        spans.push(span);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

pub(crate) fn render_status(model: &Model, frame: &mut Frame, area: Rect) {
    let theme = &model.theme;
    let base = theme.bg(theme.palette.surface);

    let mut prefix: Vec<Span<'static>> = Vec::new();
    if model.pending.is_some() {
        prefix.push(Span::styled(
            format!("{} ", spinner_frame(model.spinner)),
            theme.fg(theme.palette.accent),
        ));
        prefix.push(Span::styled(
            "projecting… ".to_owned(),
            theme.fg(theme.palette.text_dim),
        ));
    }
    let mut detail: Vec<Span<'static>> = Vec::new();
    if let Some((added, removed)) = diff_stats(model) {
        detail.push(Span::styled(
            format!("+{added} "),
            theme.fg(theme.palette.add_fg),
        ));
        detail.push(Span::styled(
            format!("−{removed} "),
            theme.fg(theme.palette.del_fg),
        ));
    } else if let Some(text) = model.active_text() {
        detail.push(Span::styled(
            format!("{} lines ", text.lines().count()),
            theme.fg(theme.palette.text_muted),
        ));
    }

    if let Some(search) = &model.search {
        let total = search.matches.len();
        let current = if total == 0 { 0 } else { search.cursor + 1 };
        detail.push(Span::styled(
            format!("/{} {current}/{total} ", clip_tail(&search.needle, 20)),
            theme.fg(theme.palette.match_fg),
        ));
    }

    let mut hints = hints(model);
    let hints_style = theme.fg(theme.palette.text_muted);
    let fixed_width: usize = prefix
        .iter()
        .chain(detail.iter())
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let area_width = area.width as usize;
    if fixed_width + UnicodeWidthStr::width(hints.as_str()) + 10 > area_width {
        hints = compact_hints(model).to_owned();
    }
    if fixed_width + UnicodeWidthStr::width(hints.as_str()) + 4 > area_width {
        hints.clear();
    }
    let hints_width = UnicodeWidthStr::width(hints.as_str());
    let path = model
        .selected
        .as_ref()
        .map_or_else(|| "no selection".to_owned(), ToString::to_string);
    let path_width = area_width.saturating_sub(fixed_width + hints_width + 3);
    let mut spans = prefix;
    spans.push(Span::styled(
        format!(" {} ", clip_tail(&path, path_width)),
        theme.fg(if model.selected.is_some() {
            theme.palette.text
        } else {
            theme.palette.text_muted
        }),
    ));
    spans.extend(detail);

    if hints_width > 0 {
        let left_width: usize = spans
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
            .sum();
        spans.push(Span::styled(
            " ".repeat(area_width.saturating_sub(left_width + hints_width)),
            base,
        ));
        spans.push(Span::styled(hints, hints_style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

fn compact_hints(model: &Model) -> &'static str {
    if model.overlay.is_some() {
        "Esc close "
    } else {
        "? help "
    }
}

fn hints(model: &Model) -> String {
    match &model.overlay {
        Some(Overlay::Revision { .. }) => "Enter apply   Esc cancel ".to_owned(),
        Some(Overlay::Scope(_)) => "↑↓ choose   Enter select   Esc close ".to_owned(),
        Some(Overlay::Mode { .. }) => "↑↓ choose   Enter apply   Esc close ".to_owned(),
        Some(Overlay::Palette(_)) | Some(Overlay::Finder(_)) => {
            "type to filter   ↑↓ choose   Enter open   Esc close ".to_owned()
        }
        Some(Overlay::Search(_)) => "type to search   Enter next   Esc cancel ".to_owned(),
        Some(Overlay::Help) => "Esc close ".to_owned(),
        None => match (model.focus, &model.content) {
            (Pane::Commits, _) => {
                "j/k choose commit   Tab files   Ctrl-P commands   ? help ".to_owned()
            }
            (Pane::Tree, _) => {
                "click open   Ctrl-P commands   Ctrl-F find   Tab content   ? help ".to_owned()
            }
            (Pane::Body, _) => {
                "wheel / j k scroll   / search   Ctrl-P commands   Tab tree   ? help ".to_owned()
            }
            (Pane::Diff, _) => {
                "wheel / j k scroll   / search   n/N match   Ctrl-P commands   ? help ".to_owned()
            }
        },
    }
}

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
            crate::app::VisualRowKind::Diff(base::DiffRowKind::Add) => added += 1,
            crate::app::VisualRowKind::Diff(base::DiffRowKind::Delete) => removed += 1,
            crate::app::VisualRowKind::Diff(base::DiffRowKind::Change) => {
                added += 1;
                removed += 1;
            }
            crate::app::VisualRowKind::Diff(base::DiffRowKind::Equal)
            | crate::app::VisualRowKind::Hunk
            | crate::app::VisualRowKind::Collapse(_) => {}
        }
    }
    Some((added, removed))
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(frame: u8) -> &'static str {
    SPINNER[frame as usize % SPINNER.len()]
}

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
                .fg_bg(theme.ink(theme.palette.danger), theme.palette.danger)
                .add_modifier(Modifier::BOLD),
        ),
        popup,
    );
}

pub(crate) fn render_too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(
        Paragraph::new("terminal too small")
            .style(theme.fg_bg(theme.palette.text_muted, theme.palette.bg)),
        area,
    );
}

fn chip(label: &str, background: Rgb, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        theme.fg_bg(theme.ink(background), background),
    )
}

// Keeps the tail of a path so the deepest directory stays visible.
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
