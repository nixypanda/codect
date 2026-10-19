//! The scope chooser plugin.
//!
//! `s` opens a chooser over the configured areas. Opening it is two steps: the
//! overlay appears in its loading state and the plugin asks the runtime for the
//! areas through its own `LoadAreas` effect. The result arrives as
//! [`Msg::AreasLoaded`] and is only applied while the chooser is open.

use base::{AreaSet, RepoPath, Selection, SelectionGroup};
use engine::{Engine, EngineError};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::component::text_input::{Edit, TextInput};
use crate::kernel::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq, Owner};
use crate::kernel::cx::Cx;
use crate::kernel::help::{HelpEntry, VIEW};
use crate::kernel::plugin::{
    Event, HintTarget, Input, Out, OverlaySpec, PaneId, Plugin, PluginId, QueryChange, Reply,
    Request,
};
use crate::plugin::SCOPE_PANE;
use crate::render::block::{popup_block, scrim};
use crate::util::input::Key;

/// The plugin's id.
pub const SCOPE: PluginId = PluginId("scope");

/// The scope chooser's effect: load the configured areas.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    LoadAreas,
}

/// The result of [`Cmd::LoadAreas`].
#[derive(Debug)]
pub enum Msg {
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

/// The scope chooser's state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeChooser {
    pub areas: Option<AreaSet>,
    pub cursor: usize,
    pub input: Option<TextInput>,
    pub error: Option<String>,
}

impl ScopeChooser {
    fn loading() -> Self {
        Self {
            areas: None,
            cursor: 0,
            input: None,
            error: None,
        }
    }

    fn options(&self) -> Vec<String> {
        let mut options = vec!["all".to_owned()];
        if let Some(areas) = &self.areas {
            options.extend(areas.names().map(str::to_owned));
        }
        options.push("path…".to_owned());
        options
    }

    fn area_count(&self) -> usize {
        self.areas.as_ref().map_or(0, |areas| areas.names().count())
    }
}

impl Default for ScopeChooser {
    fn default() -> Self {
        Self::loading()
    }
}

fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CommandId {
            owner: Owner::Scope,
            name: "open",
        },
        label: "Change scope",
        hint: "s",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::Char('s')),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank: 110,
    }]
}

impl Plugin for ScopeChooser {
    const ID: PluginId = SCOPE;
    type Msg = Msg;
    type Cmd = Cmd;

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        vec![HelpEntry {
            section: VIEW,
            keys: "s",
            text: "change scope",
            rank: 30,
        }]
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == SCOPE_PANE {
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
            HintTarget::Overlay(pane) if pane == SCOPE_PANE => {
                Some("↑↓ choose   Enter select   Esc close ")
            }
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane == SCOPE_PANE {
            render_scope(self, area, cx, frame);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<Cmd> {
        if pane != SCOPE_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        Reply::handled(self.key(key))
    }

    fn run(&mut self, command: CommandId, _cx: &Cx) -> Vec<Out<Cmd>> {
        if command.owner != Owner::Scope {
            return Vec::new();
        }
        match command.name {
            "open" => {
                *self = Self::loading();
                vec![
                    Out::Kernel(Request::OpenOverlay(SCOPE_PANE)),
                    Out::Effect(Cmd::LoadAreas),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn update(&mut self, msg: Msg, cx: &Cx) -> Vec<Out<Cmd>> {
        let Msg::AreasLoaded(result) = msg;
        if cx.overlay != Some(SCOPE_PANE) {
            return Vec::new();
        }
        match result {
            Ok(areas) => {
                self.cursor = self.cursor.min(areas.names().count());
                self.areas = Some(areas);
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        Vec::new()
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<Cmd>> {
        if let Event::OverlayClosed(pane) = event
            && *pane == SCOPE_PANE
        {
            *self = Self::default();
        }
        Vec::new()
    }

    fn perform(engine: &Engine, cmd: Cmd) -> Msg {
        match cmd {
            Cmd::LoadAreas => Msg::AreasLoaded(engine.load_areas().map_err(Box::new)),
        }
    }

    fn startup_failure(msg: Msg) -> Result<Msg, Box<EngineError>> {
        Ok(msg)
    }
}

impl ScopeChooser {
    fn key(&mut self, key: Key) -> Vec<Out<Cmd>> {
        match key {
            Key::Esc => {
                if self.input.is_some() {
                    self.input = None;
                    self.error = None;
                    Vec::new()
                } else {
                    vec![Out::Kernel(Request::CloseOverlay(SCOPE_PANE))]
                }
            }
            Key::Enter if self.input.is_some() => {
                let Some(input) = self.input.take() else {
                    return Vec::new();
                };
                match RepoPath::new(input.value()) {
                    Ok(path) => vec![
                        Out::Kernel(Request::CloseOverlay(SCOPE_PANE)),
                        Out::Kernel(Request::Requery(QueryChange::Scope(Selection::new(vec![
                            SelectionGroup::Path(path),
                        ])))),
                    ],
                    Err(error) => {
                        self.error = Some(error.to_string());
                        self.input = Some(input);
                        Vec::new()
                    }
                }
            }
            Key::Enter => {
                let area_count = self.area_count();
                let cursor = self.cursor.min(area_count + 1);
                if cursor == 0 {
                    vec![
                        Out::Kernel(Request::CloseOverlay(SCOPE_PANE)),
                        Out::Kernel(Request::Requery(QueryChange::Scope(Selection::all()))),
                    ]
                } else if cursor <= area_count {
                    let area = self.areas.as_ref().and_then(|areas| {
                        let name = areas.names().nth(cursor - 1)?;
                        areas.get(name)
                    });
                    match area.cloned() {
                        Some(area) => vec![
                            Out::Kernel(Request::CloseOverlay(SCOPE_PANE)),
                            Out::Kernel(Request::Requery(QueryChange::Scope(Selection::new(
                                vec![SelectionGroup::Area(area)],
                            )))),
                        ],
                        None => Vec::new(),
                    }
                } else {
                    self.input = Some(TextInput::new(""));
                    Vec::new()
                }
            }
            Key::Up | Key::Char('k') if self.input.is_none() => {
                self.cursor = self.cursor.saturating_sub(1);
                Vec::new()
            }
            Key::Down | Key::Char('j') if self.input.is_none() => {
                let last = self.options().len().saturating_sub(1);
                self.cursor = (self.cursor + 1).min(last);
                Vec::new()
            }
            key if self.input.is_some() => {
                if let Some(edit) = text_edit(key)
                    && let Some(input) = &mut self.input
                {
                    input.edit(edit);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

fn text_edit(key: Key) -> Option<Edit> {
    match key {
        Key::Char(character) => Some(Edit::Insert(character)),
        Key::Backspace => Some(Edit::Backspace),
        Key::Delete => Some(Edit::Delete),
        Key::Left => Some(Edit::Left),
        Key::Right => Some(Edit::Right),
        Key::Home => Some(Edit::Home),
        Key::End => Some(Edit::End),
        _ => None,
    }
}

fn render_scope(chooser: &ScopeChooser, area: Rect, cx: &Cx, frame: &mut Frame) {
    let theme = cx.theme;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::app::{Msg as KernelMsg, update};
    use crate::test_support::{area_set, press, scope_msg, selected, two_files};

    #[test]
    fn s_opens_the_loading_chooser_and_requests_areas() {
        let app = two_files();
        let (opened, cmds) = update(KernelMsg::Key(Key::Char('s')), &app);
        assert_eq!(opened.kernel.overlay, Some(SCOPE_PANE));
        assert!(opened.plugins.scope.areas.is_none());
        assert_eq!(
            cmds,
            vec![crate::plugins::PluginsCmd::Scope(Cmd::LoadAreas)]
        );
    }

    #[test]
    fn areas_apply_only_while_the_chooser_is_open() {
        let app = two_files();
        let (closed, _) = update(KernelMsg::Key(Key::Char('s')), &app);
        let (closed, _) = update(KernelMsg::Key(Key::Esc), &closed);
        assert_eq!(closed.kernel.overlay, None);
        let (stale, _) = update(scope_msg(Msg::AreasLoaded(Ok(area_set()))), &closed);
        assert!(stale.plugins.scope.areas.is_none());
    }

    #[test]
    fn enter_on_a_path_selection_requeries_with_that_path() {
        let app = two_files();
        let opened = press(&app, &[Key::Char('s')]);
        let loaded =
            crate::test_support::step(&opened, scope_msg(Msg::AreasLoaded(Ok(area_set()))));
        let mut state = loaded;
        for _ in 0..3 {
            state = press(&state, &[Key::Down]);
        }
        let input = press(&state, &[Key::Enter]);
        let typed = press(
            &input,
            &"src/lib.rs".chars().map(Key::Char).collect::<Vec<_>>(),
        );
        let (chosen, _) = update(KernelMsg::Key(Key::Enter), &typed);
        assert_eq!(chosen.kernel.overlay, None);
        assert_eq!(selected(&chosen).as_deref(), Some("a.rs"));
    }
}
