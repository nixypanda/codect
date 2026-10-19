//! The help plugin: the static key reference in the overlay slot.
//!
//! The rows are the historical help text, kept verbatim so the frame stays
//! stable. `?` opens the popup and `?` or `Esc` closes it; every other key is
//! swallowed while it is open.

use engine::Engine;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::kernel::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq, Owner};
use crate::kernel::cx::Cx;
use crate::kernel::plugin::{
    Event, HintTarget, Input, Never, Out, OverlaySpec, PaneId, Plugin, PluginId, Reply, Request,
};
use crate::plugin::HELP_PANE;
use crate::render::block::{popup_block, scrim};
use crate::render::layout::centered;
use crate::render::theme::Theme;
use crate::util::input::Key;

/// The plugin's id.
pub const HELP: PluginId = PluginId("help");

/// The help plugin's state. The popup is static; only its presence matters.
#[derive(Clone, Debug, Default)]
pub struct Help;

/// The help command, shared with the legacy palette bridge.
pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CommandId {
            owner: Owner::Help,
            name: "help.toggle",
        },
        label: "Help",
        hint: "?",
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::Char('?')),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank: 380,
    }]
}

impl Plugin for Help {
    const ID: PluginId = HELP;
    type Msg = Never;
    type Cmd = Never;

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn overlay_spec(&self, pane: PaneId) -> OverlaySpec {
        if pane == HELP_PANE {
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
            HintTarget::Overlay(pane) if pane == HELP_PANE => Some("Esc close "),
            HintTarget::Overlay(_) | HintTarget::Focus(_) => None,
        }
    }

    fn render_overlay(&self, pane: PaneId, area: Rect, cx: &Cx, frame: &mut Frame) {
        if pane == HELP_PANE {
            render_help(frame, area, cx.theme);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, _cx: &Cx) -> Reply<Never> {
        if pane != HELP_PANE {
            return Reply::unhandled();
        }
        let Input::Key(key) = input else {
            return Reply::unhandled();
        };
        match key {
            Key::Char('?') | Key::Esc => {
                Reply::handled(vec![Out::Kernel(Request::CloseOverlay(HELP_PANE))])
            }
            _ => Reply::handled(Vec::new()),
        }
    }

    fn run(&mut self, command: CommandId, _cx: &Cx) -> Vec<Out<Never>> {
        if command.owner != Owner::Help {
            return Vec::new();
        }
        match command.name {
            "help.toggle" => vec![Out::Kernel(Request::OpenOverlay(HELP_PANE))],
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

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let popup = centered(area, 56, 31);
    if popup.width == 0 || popup.height == 0 {
        return;
    }
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("help", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines = vec![
        Line::from(section("Move", theme)),
        key_line("↑ ↓ / k j", "move the tree or body cursor", theme),
        key_line("← → / h l", "fold the tree or pan sideways", theme),
        key_line("g / G", "jump to the top or bottom", theme),
        Line::from(""),
        Line::from(section("Fold", theme)),
        key_line("Enter / za", "toggle the fold at the cursor", theme),
        key_line("zo / zc", "open or close the fold", theme),
        key_line("zO / zC", "open or close recursively", theme),
        key_line("zR / zM", "unfold all / fold all", theme),
        Line::from(""),
        Line::from(section("Find", theme)),
        key_line("Ctrl-P", "open the command palette", theme),
        key_line("Ctrl-F", "find a file by name", theme),
        key_line("/ then n / N", "search the view and step matches", theme),
        Line::from(""),
        Line::from(section("View", theme)),
        key_line("Tab", "switch tree and content", theme),
        key_line("m / s", "switch mode or change scope", theme),
        key_line("r / b / t", "edit the revision, base, or target", theme),
        key_line("[ / ] / \\", "resize or reset the tree", theme),
        key_line("? / Esc", "toggle help or dismiss", theme),
        key_line("q / Ctrl-C", "quit", theme),
        Line::from(""),
        Line::from(section("Mouse", theme)),
        key_line("click", "select a file or fold a directory", theme),
        key_line("gutter + / -", "fold or unfold a declaration", theme),
        key_line("click line", "move the source cursor", theme),
        key_line("wheel", "scroll the tree or the content", theme),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn section(title: &str, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {title}"),
        theme.fg(theme.palette.accent).add_modifier(Modifier::BOLD),
    )
}

fn key_line(keys: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("   {keys:<12}"), theme.fg(theme.palette.text)),
        Span::styled(description.to_owned(), theme.fg(theme.palette.text_dim)),
    ])
}
