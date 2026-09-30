use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

use super::text::truncate_ellipsis;

pub(super) fn render_empty(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    detail: &str,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let width = area.width as usize;
    let mut lines = vec![Line::from(Span::styled(
        truncate_ellipsis(title, width),
        theme.fg(theme.palette.text),
    ))];
    if area.height > 1 {
        lines.push(Line::from(Span::styled(
            truncate_ellipsis(detail, width),
            theme.fg(theme.palette.text_muted),
        )));
    }
    let message_height = lines.len() as u16;
    let centered = Rect {
        y: area.y + (area.height - message_height) / 2,
        height: message_height,
        ..area
    };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), centered);
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use super::*;
    use crate::theme::{Capability, Flavor};

    fn draw(width: u16, height: u16, theme: Theme) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                render_empty(
                    frame,
                    Rect::new(0, 0, width, height),
                    "No files",
                    "No projected file content in this scope.",
                    &theme,
                );
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .filter_map(|x| buffer.cell((x, y)))
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn message_is_centered_and_clipped_to_a_narrow_pane() {
        let buffer = draw(10, 5, Theme::dark());
        assert_eq!(row(&buffer, 1), " No files ");
        assert!(row(&buffer, 2).contains('…'));
        assert_eq!(row(&buffer, 0), "          ");
        assert_eq!(row(&buffer, 4), "          ");
    }

    #[test]
    fn short_pane_shows_the_title_without_color() {
        let buffer = draw(10, 1, Theme::new(Flavor::Dark, Capability::NoColor));
        assert_eq!(row(&buffer, 0), " No files ");
        assert_eq!(buffer.cell((1, 0)).expect("cell").fg, Color::Reset);
    }
}
