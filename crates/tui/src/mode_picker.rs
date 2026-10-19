//! The mode picker plugin.
//!
//! `m` opens a picker over `available_modes()`. `Enter` asks the active view to
//! requery; the committed mode only changes when that load succeeds, so the
//! picker itself never mutates the workspace.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::kernel::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq, Owner};
use crate::kernel::cx::Cx;
use crate::kernel::help::{HelpEntry, VIEW};
use crate::kernel::plugin::{
    Event, HintTarget, Input, Never, Out, OverlaySpec, PaneId, Plugin, PluginId, QueryChange,
    Reply, Request,
};
use crate::kernel::workspace::{available_modes, mode_label};
use crate::plugin::MODES_PANE;
use crate::render::block::{popup_block, scrim};
use crate::util::input::Key;

/// The plugin's id.
pub const MODES: PluginId = PluginId("modes");

/// The mode picker's state: the highlighted row.
#[derive(Clone, Debug, Default)]
pub struct ModePicker {
    pub cursor: usize,
}

fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CommandId {
            owner: Owner::Modes,
            name: "open",
        },
        label: "Switch types / signatures / tests",
        hint: "m",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::Char('m')),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank: 100,
    }]
}

impl Plugin for ModePicker {
    const ID: PluginId = MODES;
    type Msg = Never;
    type Cmd = Never;

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        vec![HelpEntry {
            section: VIEW,
            keys: "m",
            text: "switch mode",
            rank: 20,
        }]
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == MODES_PANE {
            OverlaySpec {
                captures_text: false,
                modal: true,
                keeps_ctrl_jk: false,
            }
        } else {
            OverlaySpec::MODAL
        }
    }

    fn hints(&self, target: HintTarget, _cx: &Cx) -> Option<&'static str> {
        match target {
            HintTarget::Overlay(pane) if pane == MODES_PANE => {
                Some("↑↓ choose   Enter apply   Esc close ")
            }
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane == MODES_PANE {
            render_mode(self.cursor, cx.workspace.query.mode, area, cx, frame);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<Never> {
        if pane != MODES_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        let modes = available_modes();
        let outs = match key {
            Key::Esc => vec![Out::Kernel(Request::CloseOverlay(MODES_PANE))],
            Key::Up | Key::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                Vec::new()
            }
            Key::Down | Key::Char('j') => {
                self.cursor = (self.cursor + 1).min(modes.len() - 1);
                Vec::new()
            }
            Key::Enter => {
                let mode = modes[self.cursor.min(modes.len() - 1)];
                vec![
                    Out::Kernel(Request::CloseOverlay(MODES_PANE)),
                    Out::Kernel(Request::Requery(QueryChange::Mode(mode))),
                ]
            }
            _ => Vec::new(),
        };
        Reply::handled(outs)
    }

    fn run(&mut self, command: CommandId, cx: &Cx) -> Vec<Out<Never>> {
        if command.owner != Owner::Modes {
            return Vec::new();
        }
        match command.name {
            "open" => {
                let modes = available_modes();
                self.cursor = modes
                    .iter()
                    .position(|mode| *mode == cx.workspace.query.mode)
                    .unwrap_or(0);
                vec![Out::Kernel(Request::OpenOverlay(MODES_PANE))]
            }
            _ => Vec::new(),
        }
    }

    fn on_event(&mut self, _event: &Event, _cx: &Cx) -> Vec<Out<Never>> {
        Vec::new()
    }

    fn perform(_engine: &Engine, cmd: Never) -> Never {
        match cmd {}
    }
}

fn render_mode(
    cursor: usize,
    current: base::ProjectionMode,
    area: Rect,
    cx: &Cx,
    frame: &mut Frame,
) {
    let theme = cx.theme;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::app::{Msg, update};
    use crate::test_support::{press, two_files};

    #[test]
    fn m_opens_at_the_current_mode_and_enter_requeries() {
        let app = two_files();
        let opened = press(&app, &[Key::Char('m')]);
        assert_eq!(opened.kernel.overlay, Some(MODES_PANE));
        assert_eq!(opened.plugins.modes.cursor, 0);

        let down = press(&opened, &[Key::Down]);
        let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);
        assert_eq!(chosen.kernel.overlay, None);
        assert!(matches!(
            cmds.as_slice(),
            [crate::plugins::PluginsCmd::Source(_)]
        ));
    }

    #[test]
    fn j_and_k_move_the_cursor_within_the_modes() {
        let app = two_files();
        let opened = press(&app, &[Key::Char('m')]);
        let down = press(&opened, &[Key::Char('j')]);
        assert_eq!(down.plugins.modes.cursor, 1);
        let bottom = press(&down, &[Key::Char('j'), Key::Char('j')]);
        assert_eq!(bottom.plugins.modes.cursor, 2, "clamped at the last mode");
        let up = press(&bottom, &[Key::Char('k')]);
        assert_eq!(up.plugins.modes.cursor, 1);
    }
}
