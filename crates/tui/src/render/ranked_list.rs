//! The fuzzy-ranked list shared by the command palette and the file finder.
//!
//! A [`Ranked`] indexes the candidate list; the renderer highlights the matched
//! byte offsets and right-aligns a hint when it fits. The two overlays own their
//! own candidate lists and state; only the drawing is shared here.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::render::theme::Theme;

/// One fuzzy match: the candidate index, its score, and the matched byte
/// offsets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ranked {
    pub index: usize,
    pub score: i32,
    pub positions: Vec<usize>,
}

pub(crate) fn ranked_line(
    label: &str,
    hint: &str,
    positions: &[usize],
    selected: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let base = if selected {
        theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
    } else {
        Style::default()
    };
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_matched = false;
    for (byte, character) in label.char_indices() {
        let matched = positions.binary_search(&byte).is_ok();
        if matched != current_matched && !current.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut current),
                matched_style(current_matched, selected, theme),
            ));
        }
        current_matched = matched;
        current.push(character);
    }
    if !current.is_empty() {
        spans.push(Span::styled(
            current,
            matched_style(current_matched, selected, theme),
        ));
    }

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let hint_width = UnicodeWidthStr::width(hint);
    if used + hint_width + 2 <= width {
        spans.push(Span::styled(" ".repeat(width - used - hint_width), base));
        spans.push(Span::styled(
            hint.to_owned(),
            theme.fg_bg(
                theme.palette.text_muted,
                if selected {
                    theme.palette.selection_bg
                } else {
                    theme.palette.bg
                },
            ),
        ));
    }
    Line::from(spans)
}

fn matched_style(matched: bool, selected: bool, theme: &Theme) -> Style {
    if !matched {
        return if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
    }
    if selected {
        theme.fg_bg(
            theme.ink(theme.palette.match_current_bg),
            theme.palette.match_current_bg,
        )
    } else {
        theme
            .fg(theme.palette.match_fg)
            .add_modifier(Modifier::BOLD)
    }
}
