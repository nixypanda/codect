//! The search input plugin: the `/` overlay that edits the workspace search.
//!
//! Typing updates the committed search live through `SetSearch`; `Enter`
//! commits and reveals the current match, `Esc` cancels and clears it.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::plugin::SEARCH_PANE;
use tui_core::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq};
use tui_core::cx::Cx;
use tui_core::help::{FIND, HelpEntry};
use tui_core::plugin::{
    Event, HintTarget, Input, Never, Out, OverlaySpec, PaneId, Plugin, PluginId, Reply, Request,
};
use tui_core::util::input::Key;
use tui_core::widgets::text_input::{Edit, TextInput, input_line};

/// The plugin's id.
pub const SEARCH: PluginId = PluginId("search");

/// The search input's state: the editable needle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchInput {
    pub input: TextInput,
}

impl SearchInput {
    fn reset(&mut self) {
        self.input = TextInput::new("");
    }
}

fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CommandId {
            owner: SEARCH,
            name: "open",
        },
        label: "Search in view",
        hint: "/",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::Char('/')),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank: 310,
    }]
}

impl Plugin for SearchInput {
    const ID: PluginId = SEARCH;
    type Msg = Never;
    type Cmd = Never;

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        vec![HelpEntry {
            section: FIND,
            keys: "/ then n / N",
            text: "search the view and step matches",
            rank: 30,
        }]
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == SEARCH_PANE {
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
            HintTarget::Overlay(pane) if pane == SEARCH_PANE => {
                Some("type to search   Enter next   Esc cancel ")
            }
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane == SEARCH_PANE {
            render_search(self, area, cx, frame);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<Never> {
        if pane != SEARCH_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        let outs = match key {
            Key::Esc => vec![
                Out::Kernel(Request::CloseOverlay(SEARCH_PANE)),
                Out::Kernel(Request::SetSearch(String::new())),
            ],
            Key::Enter => vec![
                Out::Kernel(Request::CloseOverlay(SEARCH_PANE)),
                Out::Kernel(Request::CommitSearch),
            ],
            Key::Char(character) => {
                self.input.edit(Edit::Insert(character));
                vec![Out::Kernel(Request::SetSearch(self.input.value()))]
            }
            Key::Backspace => {
                self.input.edit(Edit::Backspace);
                vec![Out::Kernel(Request::SetSearch(self.input.value()))]
            }
            _ => Vec::new(),
        };
        Reply::handled(outs)
    }

    fn run(&mut self, command: CommandId, _cx: &Cx) -> Vec<Out<Never>> {
        if command.owner != SEARCH {
            return Vec::new();
        }
        match command.name {
            "open" => {
                self.reset();
                vec![Out::Kernel(Request::OpenOverlay(SEARCH_PANE))]
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

fn render_search(state: &SearchInput, area: Rect, cx: &Cx, frame: &mut Frame) {
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
    tui_core::render::block::scrim(frame, area);
    frame.render_widget(Clear, popup);

    let mut spans = input_line("/ ", &state.input, cx.theme).spans;
    let count = cx
        .workspace
        .search
        .as_ref()
        .map_or(0, |search| search.matches.len());
    let summary = if state.input.value().is_empty() {
        "  type to search".to_owned()
    } else if count == 0 {
        "  no matches".to_owned()
    } else {
        format!("  {count} matches")
    };
    spans.push(Span::styled(
        summary,
        cx.theme.fg(cx.theme.palette.text_muted),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(cx.theme.bg(cx.theme.palette.surface)),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{press, show_app, source_outline};
    use tui_core::app::{Msg, update};

    #[test]
    fn typing_sets_the_search_live_and_enter_commits() {
        let app = show_app(vec![source_outline("a.rs", "alpha\nbeta\nalpha\n")]);
        let opened = press(&app, &[Key::Char('/')]);
        assert_eq!(opened.kernel.overlay, Some(SEARCH_PANE));
        let typed = press(&opened, &[Key::Char('a'), Key::Char('l')]);
        assert_eq!(
            typed.workspace.search.as_ref().expect("live search").needle,
            "al"
        );
        let (committed, _) = update(Msg::Key(Key::Enter), &typed);
        assert_eq!(committed.kernel.overlay, None);
        assert!(committed.workspace.search.is_some());
    }

    #[test]
    fn esc_closes_and_clears_the_search() {
        let app = show_app(vec![source_outline("a.rs", "alpha\n")]);
        let opened = press(&app, &[Key::Char('/')]);
        let typed = press(&opened, &[Key::Char('a')]);
        let (closed, _) = update(Msg::Key(Key::Esc), &typed);
        assert_eq!(closed.kernel.overlay, None);
        assert!(closed.workspace.search.is_none());
    }
}
