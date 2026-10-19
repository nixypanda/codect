//! The which-key band plugin.
//!
//! While a `Prefixed(prefix, _)` binding is armed, the kernel broadcasts
//! `PrefixPending`; this plugin opens its overlay pane and draws the group's
//! bindings. `↑`/`↓` and Ctrl-K/Ctrl-J move the highlight, `Enter` runs the
//! highlighted command; every other key is unhandled, so the kernel resolves it
//! against the keymap (section 4.8 step 4).
//!
//! The band reads everything else from [`Cx`]: the pending prefix, the command
//! registry, and the current focus.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::kernel::command::PrefixGroupView;
use crate::kernel::cx::Cx;
use crate::kernel::plugin::{
    Event, HintTarget, Input, Never, Out, OverlaySpec, PaneId, Plugin, PluginId, Reply, Request,
};
use crate::plugin::WHICH_KEY_PANE;
use crate::render::block::popup_block;
use crate::render::layout::window_offset;
use crate::render::theme::Theme;
use crate::util::input::Key;

/// The plugin's id.
pub const WHICH_KEY: PluginId = PluginId("which-key");

/// The which-key band plugin's state.
#[derive(Clone, Default)]
pub struct WhichKey {
    pub cursor: usize,
}

impl WhichKey {
    fn bindings(cx: &Cx) -> Option<PrefixGroupView> {
        cx.prefix_group(cx.pending_prefix?)
    }
}

impl Plugin for WhichKey {
    const ID: PluginId = WHICH_KEY;
    type Msg = Never;
    type Cmd = Never;

    fn hints(&self, target: HintTarget, _cx: &Cx) -> Option<&'static str> {
        match target {
            HintTarget::Overlay(pane) if pane == WHICH_KEY_PANE => {
                Some("↑↓ / Ctrl-K/J choose   Enter run   Esc close ")
            }
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == WHICH_KEY_PANE {
            OverlaySpec {
                captures_text: true,
                modal: false,
                keeps_ctrl_jk: true,
            }
        } else {
            OverlaySpec::MODAL
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane != WHICH_KEY_PANE {
            return;
        }
        let Some(group) = Self::bindings(cx) else {
            return;
        };
        render_band(self.cursor, &group, area, cx.theme, frame);
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Never> {
        if pane != WHICH_KEY_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        match key {
            Key::Up | Key::CtrlK => {
                self.cursor = self.cursor.saturating_sub(1);
                Reply::handled(Vec::new())
            }
            Key::Down | Key::CtrlJ => {
                let last =
                    Self::bindings(cx).map_or(0, |group| group.bindings.len().saturating_sub(1));
                self.cursor = (self.cursor + 1).min(last);
                Reply::handled(Vec::new())
            }
            Key::Enter => {
                let highlighted = Self::bindings(cx)
                    .and_then(|group| group.bindings.get(self.cursor).map(|(_, _, id)| *id));
                let mut outs = vec![Out::Kernel(Request::CancelPrefix)];
                if let Some(id) = highlighted {
                    outs.push(Out::Kernel(Request::RunCommand(id)));
                }
                Reply::handled(outs)
            }
            _ => Reply::unhandled(),
        }
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<Never>> {
        match event {
            Event::PrefixPending(_) => {
                self.cursor = 0;
                vec![Out::Kernel(Request::OpenOverlay(WHICH_KEY_PANE))]
            }
            Event::PrefixCleared => vec![Out::Kernel(Request::CloseOverlay(WHICH_KEY_PANE))],
            Event::OverlayClosed(_) => Vec::new(),
            _ => Vec::new(),
        }
    }

    fn perform(_engine: &Engine, cmd: Never) -> Never {
        match cmd {}
    }
}

fn render_band(
    cursor: usize,
    group: &PrefixGroupView,
    area: Rect,
    theme: &Theme,
    frame: &mut Frame,
) {
    // Keep the top header row and the bottom status row; the band sits between.
    let available = area.height.saturating_sub(2) as usize;
    if group.bindings.is_empty() || area.width < 4 || available < 3 {
        return;
    }

    let entry_width = group
        .bindings
        .iter()
        .map(|(_, label, _)| 2 + UnicodeWidthStr::width(*label))
        .max()
        .unwrap_or(0);
    let column = entry_width + 3;
    let columns = ((area.width as usize - 2) / column).max(1);
    let rows = group.bindings.len().div_ceil(columns);
    let height = (rows + 2).min(available) as u16;
    let visible_rows = height as usize - 2;

    let band = Rect {
        x: area.x,
        y: area.y + area.height - 1 - height,
        width: area.width,
        height,
    };
    frame.render_widget(Clear, band);
    let block = popup_block(&format!("{} ({})", group.name, group.key), theme);
    let inner = block.inner(band);
    frame.render_widget(block, band);

    let cursor_row = cursor / columns;
    let offset = window_offset(cursor_row, rows, visible_rows);
    let mut lines = Vec::with_capacity(visible_rows);
    for row in offset..(offset + visible_rows).min(rows) {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for slot in 0..columns {
            let index = row * columns + slot;
            let Some((key, label, _)) = group.bindings.get(index) else {
                break;
            };
            let selected = index == cursor;
            let text = format!("{key} {label}");
            let width = UnicodeWidthStr::width(text.as_str());
            let style = if selected {
                theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
            } else {
                theme.fg(theme.palette.text)
            };
            spans.push(Span::styled(text, style));
            if slot + 1 < columns {
                spans.push(Span::raw(" ".repeat(column.saturating_sub(width))));
            }
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
