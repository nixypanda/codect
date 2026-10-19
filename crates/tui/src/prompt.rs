//! The prompt plugin: one single-line text prompt in the overlay slot.
//!
//! A plugin that needs a value emits `Request::OpenPrompt`; the kernel
//! broadcasts `PromptRequested` with the requester's id, and this plugin opens
//! its overlay. `Enter` replies with `Request::SubmitPrompt`, `Esc` cancels;
//! every other key edits the field, so `q` never quits from a prompt.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::component::text_input::{Edit, TextInput, render_prompt};
use crate::kernel::cx::Cx;
use crate::kernel::plugin::{
    Event, HintTarget, Input, Never, Out, OverlaySpec, PaneId, Plugin, PluginId, Reply, Request,
};
use crate::plugin::PROMPT_PANE;
use crate::util::input::Key;

/// The plugin's id.
pub const PROMPT: PluginId = PluginId("prompt");

/// One open prompt: who asked, which prompt it is, and the field.
#[derive(Clone, Debug)]
pub struct Open {
    pub from: PluginId,
    pub tag: u8,
    pub label: &'static str,
    pub input: TextInput,
}

/// The prompt plugin's state.
#[derive(Clone, Debug, Default)]
pub struct Prompt {
    pub open: Option<Open>,
}

impl Prompt {
    fn take(&mut self) -> Option<Open> {
        self.open.take()
    }
}

impl Plugin for Prompt {
    const ID: PluginId = PROMPT;
    type Msg = Never;
    type Cmd = Never;

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == PROMPT_PANE {
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
            HintTarget::Overlay(pane) if pane == PROMPT_PANE => Some("Enter apply   Esc cancel "),
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane != PROMPT_PANE {
            return;
        }
        if let Some(open) = &self.open {
            render_prompt(frame, area, open.label, &open.input, cx.theme);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<Never> {
        if pane != PROMPT_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        let Some(mut open) = self.take() else {
            return Reply::unhandled();
        };
        match key {
            Key::Esc => Reply::handled(vec![Out::Kernel(Request::CloseOverlay(PROMPT_PANE))]),
            Key::Enter => {
                let value = open.input.value();
                let mut outs = vec![Out::Kernel(Request::CloseOverlay(PROMPT_PANE))];
                if !value.is_empty() {
                    outs.push(Out::Kernel(Request::SubmitPrompt {
                        to: open.from,
                        tag: open.tag,
                        value,
                    }));
                }
                Reply::handled(outs)
            }
            key => {
                if let Some(edit) = text_edit(key) {
                    open.input.edit(edit);
                }
                self.open = Some(open);
                Reply::handled(Vec::new())
            }
        }
    }

    fn on_event(&mut self, event: &Event, _cx: &Cx) -> Vec<Out<Never>> {
        match event {
            Event::PromptRequested {
                from,
                label,
                initial,
                tag,
            } => {
                self.open = Some(Open {
                    from: *from,
                    tag: *tag,
                    label,
                    input: TextInput::new(initial.clone()),
                });
                vec![Out::Kernel(Request::OpenOverlay(PROMPT_PANE))]
            }
            Event::OverlayClosed(pane) if *pane == PROMPT_PANE => {
                self.open = None;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn perform(_engine: &Engine, cmd: Never) -> Never {
        match cmd {}
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
