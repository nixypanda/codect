// The revision editors: the `show` revision prompt and the diff base/target
// prompts.
//
// They belong to the loaded projection, not to an overlay — a base prompt only
// exists on a diff and a show prompt only on a show — so they are drawn by the
// app view, using the shared popup primitives.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, Paragraph};

use crate::app::App;
use crate::component::text_input::{TextInput, input_line};
use crate::page::{DiffSide, Loaded};
use crate::render::block::{popup_block, scrim};
use crate::render::theme::Theme;

pub(crate) fn render(app: &App, frame: &mut Frame, area: Rect) {
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
    frame.render_widget(Paragraph::new(input_line("", input, theme)), inner);
}
