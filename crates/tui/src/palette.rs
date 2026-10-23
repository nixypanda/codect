//! The command palette plugin.
//!
//! `Ctrl-P` opens a fuzzy filter over the kernel's palette commands. The
//! candidates and their labels come from [`Cx::palette_commands`], so the
//! palette lists every plugin's commands without naming any plugin.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

use crate::plugin::PALETTE_PANE;
use tui_core::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq};
use tui_core::cx::Cx;
use tui_core::help::{FIND, HelpEntry};
use tui_core::layout::window_offset;
use tui_core::plugin::{
    Event, HintTarget, Input, Never, Out, OverlaySpec, PaneId, Plugin, PluginId, Reply, Request,
};
use tui_core::render::block::{popup_block, scrim};
use tui_core::util::fuzzy;
use tui_core::util::input::Key;
use tui_core::widgets::ranked_list::{Ranked, ranked_line};
use tui_core::widgets::text_input::{Edit, TextInput, input_line};

/// The plugin's id.
pub const PALETTE: PluginId = PluginId("palette");

/// The palette's editable filter, its ranked matches, and the highlighted row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Palette {
    pub input: TextInput,
    pub matches: Vec<Ranked>,
    pub cursor: usize,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            input: TextInput::new(""),
            matches: Vec::new(),
            cursor: 0,
        }
    }
}

impl Palette {
    fn reset(&mut self, cx: &Cx) {
        self.input = TextInput::new("");
        self.cursor = 0;
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &Cx) {
        let commands = cx.palette_commands();
        let needle = self.input.text.clone();
        let mut matches: Vec<Ranked> = commands
            .iter()
            .enumerate()
            .filter_map(|(index, spec)| {
                fuzzy::fuzzy(&needle, spec.label).map(|matched| Ranked {
                    index,
                    score: matched.score,
                    positions: matched.positions,
                })
            })
            .collect();
        matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        self.matches = matches;
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }

    fn selected_command(&self, cx: &Cx) -> Option<CommandId> {
        let commands = cx.palette_commands();
        self.matches
            .get(self.cursor)
            .and_then(|ranked| commands.get(ranked.index))
            .map(|spec| spec.id)
    }
}

fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CommandId {
            owner: PALETTE,
            name: "open",
        },
        label: "",
        hint: "",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::CtrlP),
            scope: KeyScope::Anywhere,
        }],
        palette: false,
        group: None,
        rank: 0,
    }]
}

impl Plugin for Palette {
    const ID: PluginId = PALETTE;
    type Msg = Never;
    type Cmd = Never;

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        vec![HelpEntry {
            section: FIND,
            keys: "Ctrl-P",
            text: "open the command palette",
            rank: 10,
        }]
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == PALETTE_PANE {
            OverlaySpec {
                captures_text: true,
                modal: true,
                keeps_ctrl_jk: false,
            }
        } else {
            OverlaySpec::MODAL
        }
    }

    fn hints(&self, target: HintTarget, _cx: &Cx) -> Option<&'static str> {
        match target {
            HintTarget::Overlay(pane) if pane == PALETTE_PANE => {
                Some("type to filter   ↑↓ choose   Enter open   Esc close ")
            }
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane == PALETTE_PANE {
            render_palette(self, area, cx, frame);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Never> {
        if pane != PALETTE_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        let outs = match key {
            Key::Esc => vec![Out::Kernel(Request::CloseOverlay(PALETTE_PANE))],
            Key::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                Vec::new()
            }
            Key::Down => {
                self.cursor = (self.cursor + 1).min(self.matches.len().saturating_sub(1));
                Vec::new()
            }
            Key::Enter => {
                let mut outs = vec![Out::Kernel(Request::CloseOverlay(PALETTE_PANE))];
                if let Some(id) = self.selected_command(cx) {
                    outs.push(Out::Kernel(Request::RunCommand(id)));
                }
                outs
            }
            Key::Char(character) => {
                self.input.edit(Edit::Insert(character));
                self.cursor = 0;
                self.refresh(cx);
                Vec::new()
            }
            Key::Backspace => {
                self.input.edit(Edit::Backspace);
                self.cursor = 0;
                self.refresh(cx);
                Vec::new()
            }
            _ => Vec::new(),
        };
        Reply::handled(outs)
    }

    fn run(&mut self, command: CommandId, cx: &Cx) -> Vec<Out<Never>> {
        if command.owner != PALETTE {
            return Vec::new();
        }
        match command.name {
            "open" => {
                self.reset(cx);
                vec![Out::Kernel(Request::OpenOverlay(PALETTE_PANE))]
            }
            _ => Vec::new(),
        }
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<Never>> {
        if let Event::OverlayClosed(pane) = event
            && *pane == PALETTE_PANE
        {
            *self = Self::default();
        }
        Vec::new()
    }

    fn perform(_engine: &Engine, cmd: Never) -> Never {
        match cmd {}
    }
}

fn render_palette(state: &Palette, area: Rect, cx: &Cx, frame: &mut Frame) {
    let commands = cx.palette_commands();
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
    let block = popup_block("commands", cx.theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = vec![input_line("› ", &state.input, cx.theme), Line::from("")];
    let offset = window_offset(state.cursor, state.matches.len(), list_height);
    for (row, ranked) in state
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let (label, hint) = commands
            .get(ranked.index)
            .map_or(("", ""), |spec| (spec.label, spec.hint));
        lines.push(ranked_line(
            label,
            hint,
            &ranked.positions,
            row == state.cursor,
            width as usize - 2,
            cx.theme,
        ));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(ratatui::text::Span::styled(
            "   no matching command",
            cx.theme.fg(cx.theme.palette.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::FINDER_PANE;
    use crate::test_support::{press, two_files};
    use tui_core::app::{Msg, update};

    #[test]
    fn ctrl_p_opens_with_every_palette_command_ranked() {
        let app = two_files();
        let opened = press(&app, &[Key::CtrlP]);
        assert_eq!(opened.kernel.overlay, Some(PALETTE_PANE));
        assert_eq!(opened.plugins.palette.matches.len(), 20);
        assert_eq!(opened.plugins.palette.cursor, 0);
    }

    #[test]
    fn typing_filters_and_enter_runs_the_highlighted_command() {
        let app = two_files();
        let opened = press(&app, &[Key::CtrlP]);
        let typed = press(&opened, &[Key::Char('f'), Key::Char('i')]);
        assert!(
            typed.plugins.palette.matches.len() < 20,
            "the filter narrows the list"
        );
        let (next, cmds) = update(Msg::Key(Key::Enter), &typed);
        assert_eq!(
            next.kernel.overlay,
            Some(FINDER_PANE),
            "the palette closes and the finder opens"
        );
        assert!(cmds.is_empty());
    }

    #[test]
    fn esc_closes_without_running_anything() {
        let app = two_files();
        let opened = press(&app, &[Key::CtrlP]);
        let (closed, cmds) = update(Msg::Key(Key::Esc), &opened);
        assert_eq!(closed.kernel.overlay, None);
        assert!(cmds.is_empty());
    }
}
