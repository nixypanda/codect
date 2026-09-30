// Modal overlays: help, the scope chooser, the mode picker, the command
// palette, the fuzzy file finder, and search. The revision prompts live in the
// loaded projection and are drawn here too.
//
// Every overlay dims the frame behind it with a scrim so the popup reads as a
// layer above the browser rather than a hole in it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::content::{DiffSide, Loaded, available_modes, mode_label};
use crate::overlay::{FinderState, Overlay, PaletteState, ScopeChooser, SearchState};
use crate::text_input::TextInput;
use crate::theme::Theme;

use super::geom::{centered, window_offset};

pub(crate) fn render_overlay(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.chrome.theme;
    match &app.overlay {
        Some(Overlay::Help) => render_help(frame, area, theme),
        Some(Overlay::Scope(chooser)) => render_scope(frame, area, chooser, theme),
        Some(Overlay::Mode { cursor }) => {
            render_mode(frame, area, *cursor, app.loaded.mode(), theme)
        }
        Some(Overlay::Palette(state)) => render_palette(frame, area, app, state, theme),
        Some(Overlay::Finder(state)) => render_finder(frame, area, app, state, theme),
        Some(Overlay::Search(state)) => render_search(frame, area, state, app, theme),
        None => render_prompt(app, frame, area),
    }
}

fn render_prompt(app: &App, frame: &mut Frame, area: Rect) {
    let theme = &app.chrome.theme;
    match &app.loaded {
        Loaded::Show(show) => {
            if let Some(input) = &show.prompt {
                render_revision(frame, area, "revision", input, theme);
            }
        }
        Loaded::Diff(diff) => {
            if let Some(prompt) = &diff.prompt {
                let label = match prompt.side {
                    DiffSide::Base => "base",
                    DiffSide::Target => "target",
                };
                render_revision(frame, area, label, &prompt.input, theme);
            }
        }
    }
}

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
            theme.fg(theme.palette.accent).add_modifier(Modifier::BOLD),
        ))
}

fn render_mode(
    frame: &mut Frame,
    area: Rect,
    cursor: usize,
    current: base::ProjectionMode,
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

fn render_revision(frame: &mut Frame, area: Rect, label: &str, input: &TextInput, theme: &Theme) {
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
    let cursor_style = theme.fg_bg(theme.ink(theme.palette.accent), theme.palette.accent);
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
    let popup = centered(area, 54, 23);
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
        key_line("g / G", "jump to the top or bottom", theme),
        Line::from(""),
        Line::from(section("Find", theme)),
        key_line("Ctrl-P", "open the command palette", theme),
        key_line("Ctrl-F", "find a file by name", theme),
        key_line("/ then n / N", "search the view and step matches", theme),
        Line::from(""),
        Line::from(section("View", theme)),
        key_line("Tab", "switch tree and content", theme),
        key_line("m / s", "switch mode or change scope", theme),
        key_line("r / b / t", "edit the revision, base, or target", theme),
        key_line("[ / ] / \\", "resize or reset the tree", theme),
        key_line("? / Esc", "toggle help or dismiss", theme),
        key_line("q / Ctrl-C", "quit", theme),
        Line::from(""),
        Line::from(section("Mouse", theme)),
        key_line("click", "open a file or fold a directory", theme),
        key_line("wheel", "scroll the tree or the content", theme),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_palette(frame: &mut Frame, area: Rect, app: &App, state: &PaletteState, theme: &Theme) {
    let entries = app.loaded.palette_entries();
    let width = area.width.saturating_sub(4).min(72);
    let list_height = state.matches.len().min(12);
    let height = (list_height + 3).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 3,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("commands", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = vec![input_line("› ", &state.input, theme), Line::from("")];
    let offset = window_offset(state.cursor, state.matches.len(), list_height);
    for (row, ranked) in state
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let (label, hint) = entries
            .get(ranked.index)
            .map_or(("", ""), |(_, label, hint)| (*label, *hint));
        lines.push(ranked_line(
            label,
            hint,
            &ranked.positions,
            row == state.cursor,
            width as usize - 2,
            theme,
        ));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no matching command",
            theme.fg(theme.palette.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_finder(frame: &mut Frame, area: Rect, app: &App, state: &FinderState, theme: &Theme) {
    let width = area.width.saturating_sub(4).min(80);
    let list_height = state.matches.len().min(14);
    let height = (list_height + 3).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 3,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("find file", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let visible = app.loaded.visible();
    let mut lines = vec![input_line("⌕ ", &state.input, theme), Line::from("")];
    let offset = window_offset(state.cursor, state.matches.len(), list_height);
    for (row, ranked) in state
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let label = visible
            .get(ranked.index)
            .map_or(String::new(), ToString::to_string);
        lines.push(ranked_line(
            &label,
            "",
            &ranked.positions,
            row == state.cursor,
            width as usize - 2,
            theme,
        ));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no matching file",
            theme.fg(theme.palette.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_search(frame: &mut Frame, area: Rect, state: &SearchState, app: &App, theme: &Theme) {
    let width = area.width.saturating_sub(4).min(80);
    if width == 0 || area.height < 2 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height - 2,
        width,
        height: 1,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);

    let mut spans = input_line("/ ", &state.input, theme).spans;
    let count = app.loaded.search().map_or(0, |search| search.matches.len());
    let summary = if state.input.value().is_empty() {
        "  type to search".to_owned()
    } else if count == 0 {
        "  no matches".to_owned()
    } else {
        format!("  {count} matches")
    };
    spans.push(Span::styled(summary, theme.fg(theme.palette.text_muted)));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.bg(theme.palette.surface)),
        popup,
    );
}

fn input_line(prefix: &str, input: &TextInput, theme: &Theme) -> Line<'static> {
    let before = &input.text[..input.cursor];
    let after = &input.text[input.cursor..];
    let cursor_style = theme.fg_bg(theme.ink(theme.palette.accent), theme.palette.accent);
    let mut spans = vec![Span::styled(
        prefix.to_owned(),
        theme.fg(theme.palette.accent),
    )];
    spans.push(Span::styled(
        before.to_owned(),
        theme.fg(theme.palette.text),
    ));
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
    Line::from(spans)
}

fn ranked_line(
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

fn section(title: &str, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {title}"),
        theme.fg(theme.palette.accent).add_modifier(Modifier::BOLD),
    )
}

fn key_line(keys: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("   {keys:<12}"), theme.fg(theme.palette.text)),
        Span::styled(description.to_owned(), theme.fg(theme.palette.text_dim)),
    ])
}
