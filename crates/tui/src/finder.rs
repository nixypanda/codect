//! The file finder plugin.
//!
//! `Ctrl-F` opens a fuzzy filter over the workspace's file paths. The paths come
//! from [`Cx`], so the finder never reaches into the tree's state.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::plugin::FINDER_PANE;
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
pub const FINDER: PluginId = PluginId("finder");

/// The finder's filter, its ranked matches, and the highlighted row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finder {
    pub input: TextInput,
    pub matches: Vec<Ranked>,
    pub cursor: usize,
}

impl Default for Finder {
    fn default() -> Self {
        Self {
            input: TextInput::new(""),
            matches: Vec::new(),
            cursor: 0,
        }
    }
}

impl Finder {
    fn reset(&mut self, cx: &Cx) {
        self.input = TextInput::new("");
        self.cursor = 0;
        self.refresh(cx);
    }

    fn refresh(&mut self, cx: &Cx) {
        let visible = &cx.workspace.files.paths;
        let needle = self.input.text.clone();
        let mut matches: Vec<Ranked> = visible
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let text = path.to_string();
                fuzzy::fuzzy(&needle, &text).map(|matched| Ranked {
                    index,
                    score: matched.score,
                    positions: matched.positions,
                })
            })
            .collect();
        matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        matches.truncate(200);
        self.matches = matches;
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }

    fn selected_path(&self, cx: &Cx) -> Option<base::RepoPath> {
        let visible = &cx.workspace.files.paths;
        self.matches
            .get(self.cursor)
            .and_then(|ranked| visible.get(ranked.index))
            .cloned()
    }
}

fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CommandId {
            owner: FINDER,
            name: "open",
        },
        label: "Find file",
        hint: "Ctrl-F",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::CtrlF),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank: 300,
    }]
}

impl Plugin for Finder {
    const ID: PluginId = FINDER;
    type Msg = Never;
    type Cmd = Never;

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        vec![HelpEntry {
            section: FIND,
            keys: "Ctrl-F",
            text: "find a file by name",
            rank: 20,
        }]
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == FINDER_PANE {
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
            HintTarget::Overlay(pane) if pane == FINDER_PANE => {
                Some("type to filter   ↑↓ choose   Enter open   Esc close ")
            }
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane == FINDER_PANE {
            render_finder(self, area, cx, frame);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Never> {
        if pane != FINDER_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        let outs = match key {
            Key::Esc => vec![Out::Kernel(Request::CloseOverlay(FINDER_PANE))],
            Key::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                Vec::new()
            }
            Key::Down => {
                self.cursor = (self.cursor + 1).min(self.matches.len().saturating_sub(1));
                Vec::new()
            }
            Key::Enter => {
                let mut outs = vec![Out::Kernel(Request::CloseOverlay(FINDER_PANE))];
                if let Some(path) = self.selected_path(cx) {
                    outs.push(Out::Kernel(Request::Select(Some(path))));
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
        if command.owner != FINDER {
            return Vec::new();
        }
        match command.name {
            "open" => {
                self.reset(cx);
                vec![Out::Kernel(Request::OpenOverlay(FINDER_PANE))]
            }
            _ => Vec::new(),
        }
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<Never>> {
        if let Event::OverlayClosed(pane) = event
            && *pane == FINDER_PANE
        {
            *self = Self::default();
        }
        Vec::new()
    }

    fn perform(_engine: &Engine, cmd: Never) -> Never {
        match cmd {}
    }
}

fn render_finder(state: &Finder, area: Rect, cx: &Cx, frame: &mut Frame) {
    let visible = &cx.workspace.files.paths;
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
    let block = popup_block("find file", cx.theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = vec![input_line("⌕ ", &state.input, cx.theme), Line::from("")];
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
            cx.theme,
        ));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no matching file",
            cx.theme.fg(cx.theme.palette.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{press, selected, two_files};
    use tui_core::app::{Msg, update};

    #[test]
    fn ctrl_f_lists_every_file_and_enter_selects_the_highlighted_one() {
        let app = two_files();
        let opened = press(&app, &[Key::CtrlF]);
        assert_eq!(opened.kernel.overlay, Some(FINDER_PANE));
        assert_eq!(opened.plugins.finder.matches.len(), 2);

        let down = press(&opened, &[Key::Down]);
        let (chosen, _) = update(Msg::Key(Key::Enter), &down);
        assert_eq!(chosen.kernel.overlay, None);
        assert_eq!(selected(&chosen).as_deref(), Some("b.rs"));
    }

    #[test]
    fn typing_filters_by_path() {
        let app = two_files();
        let opened = press(&app, &[Key::CtrlF]);
        let typed = press(&opened, &[Key::Char('b')]);
        assert_eq!(typed.plugins.finder.matches.len(), 1);
        assert_eq!(typed.plugins.finder.matches[0].index, 1);
    }
}
