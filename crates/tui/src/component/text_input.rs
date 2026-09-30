//! A single-line editable field: the one reusable leaf component.
//!
//! It owns exactly two fields and exposes editing as an [`Edit`] value, so a
//! caller cannot reach into its internals.

use ratatui::text::{Line, Span};

use crate::render::theme::Theme;

/// An edit to apply to a [`TextInput`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edit {
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// A single-line editable field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextInput {
    pub text: String,
    /// A byte offset into `text`, always on a character boundary.
    pub cursor: usize,
}

impl TextInput {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    /// Applies one edit. The cursor stays on a character boundary.
    pub fn edit(&mut self, edit: Edit) {
        match edit {
            Edit::Insert(character) => self.insert(character),
            Edit::Backspace => self.backspace(),
            Edit::Delete => self.delete(),
            Edit::Left => self.left(),
            Edit::Right => self.right(),
            Edit::Home => self.cursor = 0,
            Edit::End => self.cursor = self.text.len(),
        }
    }

    fn insert(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    fn backspace(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().last() {
            self.text.remove(index);
            self.cursor = index;
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    fn left(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().last() {
            self.cursor = index;
        }
    }

    fn right(&mut self) {
        if let Some(character) = self.text[self.cursor..].chars().next() {
            self.cursor += character.len_utf8();
        }
    }

    /// The trimmed value to apply.
    pub fn value(&self) -> String {
        self.text.trim().to_owned()
    }
}

/// Draws one editable line with a leading `prefix` and an inverted cursor cell.
///
/// Shared by the overlay inputs (palette, finder, search) and the revision
/// prompt, so the cursor rendering has exactly one owner.
pub(crate) fn input_line(prefix: &str, input: &TextInput, theme: &Theme) -> Line<'static> {
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

// Draws a revision prompt popup: a titled, scrimmed box with one editable line.
//
// Shared by the `show` revision prompt and the diff base/target prompts, which
// belong to the loaded page rather than an overlay.
pub(crate) fn render_prompt(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    label: &str,
    input: &TextInput,
    theme: &Theme,
) {
    use ratatui::widgets::{Clear, Paragraph};

    use crate::render::block::{popup_block, scrim};

    let width = area.width.saturating_sub(4).min(70);
    let height = 3.min(area.height);
    if width == 0 || height == 0 {
        return;
    }
    let popup = ratatui::layout::Rect {
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
    frame.render_widget(Paragraph::new(input_line("", input, theme)), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_land_on_char_boundaries() {
        let mut input = TextInput::new("héllo");
        input.edit(Edit::Backspace);
        assert_eq!(input.text, "héll");
        input.edit(Edit::Home);
        input.edit(Edit::Delete);
        assert_eq!(input.text, "éll");
        input.edit(Edit::Insert('x'));
        assert_eq!(input.text, "xéll");
        assert_eq!(input.cursor, 1);
        input.edit(Edit::Right);
        assert_eq!(input.cursor, 3);
        input.edit(Edit::End);
        assert_eq!(input.cursor, input.text.len());
        input.edit(Edit::Left);
        assert_eq!(input.cursor, input.text.len() - 1);
    }

    #[test]
    fn value_is_trimmed() {
        let mut input = TextInput::new("  main  ");
        assert_eq!(input.value(), "main");
        input.edit(Edit::Backspace);
        assert_eq!(input.text, "  main ");
    }
}
