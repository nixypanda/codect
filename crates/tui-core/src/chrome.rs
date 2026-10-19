//! The frame chrome: the header, the status line, the diagnostic toast, and the
//! too-small notice. The kernel draws only the frame; a plugin contributes its
//! chips and status segments. `tui-core` exposes this to `tui` and `tui-diff`
//! through [`crate::view`] and [`crate::cx`].

#![allow(dead_code)]
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::plugin::OverlaySpec;
use crate::render::text::clip_line;
use crate::render::theme::{Rgb, Theme};
use crate::search::Search;
use crate::workspace::mode_label;

/// The tone of a header chip.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChipTone {
    Accent,
    Neutral,
}

/// A header chip contributed by the kernel or a plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chip {
    pub label: String,
    pub tone: ChipTone,
}

/// The tone of a status segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SegmentTone {
    Text,
    Muted,
    Added,
    Removed,
}

/// A status segment contributed by a plugin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment {
    pub text: String,
    pub tone: SegmentTone,
}

pub fn render_header(
    root: &str,
    mode: base::ProjectionMode,
    chips: &[Chip],
    scope: &str,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let base = theme.bg(theme.palette.surface);
    let brand = Span::styled(
        " ◆ codect ",
        theme
            .fg_bg(theme.ink(theme.palette.accent), theme.palette.accent)
            .add_modifier(Modifier::BOLD),
    );
    let mut context = vec![chip(mode_label(mode), theme.palette.accent, theme)];
    context.extend(chips.iter().map(|chip| match chip.tone {
        ChipTone::Accent => chip_span(&chip.label, theme.palette.accent, theme),
        ChipTone::Neutral => chip_span(&chip.label, theme.palette.surface_alt, theme),
    }));
    context.push(chip(
        &format!("scope: {scope}"),
        theme.palette.surface_alt,
        theme,
    ));
    let area_width = area.width as usize;
    let brand_width = UnicodeWidthStr::width(brand.content.as_ref());
    let mut shown = context.len();
    let context_width = |count: usize| {
        context[..count]
            .iter()
            .map(|span| UnicodeWidthStr::width(span.content.as_ref()) + 1)
            .sum::<usize>()
    };
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
            format!(" {} ", clip_tail(root, root_width)),
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

#[allow(clippy::too_many_arguments)]
pub fn render_status(
    busy: bool,
    spinner: u8,
    selected: Option<&str>,
    segments: &[Segment],
    search: Option<&Search>,
    hints: &str,
    compact_hints: &str,
    theme: &Theme,
    frame: &mut Frame,
    area: Rect,
) {
    let base = theme.bg(theme.palette.surface);

    let mut prefix: Vec<Span<'static>> = Vec::new();
    if busy {
        prefix.push(Span::styled(
            format!("{} ", spinner_frame(spinner)),
            theme.fg(theme.palette.accent),
        ));
        prefix.push(Span::styled(
            "projecting… ".to_owned(),
            theme.fg(theme.palette.text_dim),
        ));
    }
    let mut detail: Vec<Span<'static>> = Vec::new();
    for segment in segments {
        let color = match segment.tone {
            SegmentTone::Text => theme.palette.text,
            SegmentTone::Muted => theme.palette.text_muted,
            SegmentTone::Added => theme.palette.add_fg,
            SegmentTone::Removed => theme.palette.del_fg,
        };
        detail.push(Span::styled(segment.text.clone(), theme.fg(color)));
    }

    if let Some(search) = search {
        let total = search.matches.len();
        let current = if total == 0 { 0 } else { search.cursor + 1 };
        detail.push(Span::styled(
            format!("/{} {current}/{total} ", clip_tail(&search.needle, 20)),
            theme.fg(theme.palette.match_fg),
        ));
    }

    let hints_style = theme.fg(theme.palette.text_muted);
    let fixed_width: usize = prefix
        .iter()
        .chain(detail.iter())
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let area_width = area.width as usize;
    let mut hints = hints.to_owned();
    if fixed_width + UnicodeWidthStr::width(hints.as_str()) + 10 > area_width {
        hints = compact_hints.to_owned();
    }
    if fixed_width + UnicodeWidthStr::width(hints.as_str()) + 4 > area_width {
        hints.clear();
    }
    let hints_width = UnicodeWidthStr::width(hints.as_str());
    let path = selected.map_or_else(|| "no selection".to_owned(), str::to_owned);
    let path_width = area_width.saturating_sub(fixed_width + hints_width + 3);
    let mut spans = prefix;
    spans.push(Span::styled(
        format!(" {} ", clip_tail(&path, path_width)),
        theme.fg(if selected.is_some() {
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

/// The compact fallback hint for a target.
pub fn compact_hint(overlay: Option<OverlaySpec>) -> &'static str {
    match overlay {
        Some(_) => "Esc close ",
        None => "? help ",
    }
}

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(frame: u8) -> &'static str {
    SPINNER[frame as usize % SPINNER.len()]
}

pub fn render_diagnostic(text: Option<&str>, theme: &Theme, frame: &mut Frame, area: Rect) {
    let Some(text) = text else {
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
    frame.render_widget(
        Paragraph::new(clip_line(text, 0, width as usize)).style(
            theme
                .fg_bg(theme.ink(theme.palette.danger), theme.palette.danger)
                .add_modifier(Modifier::BOLD),
        ),
        popup,
    );
}

pub fn render_too_small(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(
        Paragraph::new("terminal too small")
            .style(theme.fg_bg(theme.palette.text_muted, theme.palette.bg)),
        area,
    );
}

fn chip(label: &str, background: Rgb, theme: &Theme) -> Span<'static> {
    chip_span(label, background, theme)
}

fn chip_span(label: &str, background: Rgb, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        theme.fg_bg(theme.ink(background), background),
    )
}

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
