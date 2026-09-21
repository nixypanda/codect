//! Modal overlays: help, revision entry, the scope chooser, and the mode picker.
//!
//! Every overlay dims the frame behind it with a scrim so the popup reads as a
//! layer above the browser rather than a hole in it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::{
    Model, Overlay, RevisionField, ScopeChooser, TextInput, available_modes, mode_label,
};
use crate::theme::Theme;

use super::geom::centered;

pub(crate) fn render_overlay(model: &Model, frame: &mut Frame, area: Rect) {
    match &model.overlay {
        Some(Overlay::Help) => render_help(frame, area, &model.theme),
        Some(Overlay::Revision { field, input }) => {
            render_revision(frame, area, *field, input, &model.theme);
        }
        Some(Overlay::Scope(chooser)) => render_scope(frame, area, chooser, &model.theme),
        Some(Overlay::Mode { cursor }) => render_mode(frame, area, *cursor, model.mode, &model.theme),
        None => {}
    }
}

/// Dims everything behind an overlay.
fn scrim(frame: &mut Frame, area: Rect) {
    frame
        .buffer_mut()
        .set_style(area, Style::default().add_modifier(Modifier::DIM));
}

fn popup_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.fg(theme.palette.border_focus))
        .style(theme.bg(theme.palette.surface))
        .title(Span::styled(
            format!(" {title} "),
            theme
                .fg(theme.palette.accent)
                .add_modifier(Modifier::BOLD),
        ))
}

fn render_mode(
    frame: &mut Frame,
    area: Rect,
    cursor: usize,
    current: ownai_core::ProjectionMode,
    theme: &Theme,
) {
    let modes = available_modes();
    let width = area.width.saturating_sub(4).min(40);
    let height = (modes.len() + 3).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("mode", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = Vec::new();
    for (index, mode) in modes.iter().enumerate() {
        let selected = index == cursor;
        let style = if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
        let marker = if *mode == current { "•" } else { " " };
        lines.push(Line::from(Span::styled(
            format!(" {marker} {} ", mode_label(*mode)),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_scope(frame: &mut Frame, area: Rect, chooser: &ScopeChooser, theme: &Theme) {
    let options = chooser.options();
    let extra = 4 + usize::from(chooser.input.is_some()) * 2 + usize::from(chooser.error.is_some());
    let width = area.width.saturating_sub(4).min(60);
    let height = (options.len() + extra).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("scope", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = Vec::new();
    if chooser.areas.is_none() && chooser.error.is_none() {
        lines.push(Line::from(Span::styled(
            " loading areas…",
            theme.fg(theme.palette.text_muted),
        )));
    }
    for (index, option) in options.iter().enumerate() {
        let selected = index == chooser.cursor && chooser.input.is_none();
        let style = if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
        lines.push(Line::from(Span::styled(format!(" {option} "), style)));
    }
    if let Some(input) = &chooser.input {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" path: ", theme.fg(theme.palette.text_dim)),
            Span::styled(input.text.clone(), theme.fg(theme.palette.text)),
        ]));
    }
    if let Some(error) = &chooser.error {
        lines.push(Line::from(Span::styled(
            format!(" {error} "),
            theme.fg(theme.palette.danger),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_revision(
    frame: &mut Frame,
    area: Rect,
    field: RevisionField,
    input: &TextInput,
    theme: &Theme,
) {
    let label = match field {
        RevisionField::Show => "revision",
        RevisionField::Base => "base",
        RevisionField::Target => "target",
    };
    let width = area.width.saturating_sub(4).min(70);
    let height = 3.min(area.height);
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height.saturating_sub(height + 1),
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block(label, theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let before = &input.text[..input.cursor];
    let after = &input.text[input.cursor..];
    let cursor_style = theme.fg_bg(theme.palette.bg, theme.palette.accent);
    let mut spans = vec![Span::styled(
        before.to_owned(),
        theme.fg(theme.palette.text),
    )];
    match after.chars().next() {
        Some(character) => {
            spans.push(Span::styled(character.to_string(), cursor_style));
            spans.push(Span::styled(
                after[character.len_utf8()..].to_owned(),
                theme.fg(theme.palette.text),
            ));
        }
        None => spans.push(Span::styled(" ", cursor_style)),
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let popup = centered(area, 52, 18);
    if popup.width == 0 || popup.height == 0 {
        return;
    }
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("help", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines = vec![
        Line::from(section("Move", theme)),
        key_line("↑ ↓ / k j", "move in the tree or scroll", theme),
        key_line("← → / h l", "fold the tree or pan sideways", theme),
        Line::from(""),
        Line::from(section("Panes", theme)),
        key_line("Tab", "switch tree and content", theme),
        key_line("Enter", "open a file or fold a directory", theme),
        Line::from(""),
        Line::from(section("View", theme)),
        key_line("m", "switch Types and Signatures", theme),
        key_line("s", "change the scope", theme),
        key_line("r / b / t", "edit the revision, base, or target", theme),
        key_line("[ / ] / \\", "resize or reset the tree", theme),
        Line::from(""),
        key_line("? / Esc", "toggle help or dismiss a diagnostic", theme),
        key_line("q / Ctrl-C", "quit", theme),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn section(title: &str, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {title}"),
        theme
            .fg(theme.palette.accent)
            .add_modifier(Modifier::BOLD),
    )
}

fn key_line(keys: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("   {keys:<12}"), theme.fg(theme.palette.text)),
        Span::styled(description.to_owned(), theme.fg(theme.palette.text_dim)),
    ])
}
