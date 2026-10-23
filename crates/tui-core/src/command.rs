//! The command vocabulary: identifiers, bindings, and the registry invariants.
//!
//! This module holds the types, the kernel-owned command table, and the section
//! 4.8 validation. `tui` and `tui-diff` declare their commands with these
//! types; the kernel contributes its own table here.

use crate::plugin::{FocusKey, PluginId};
use crate::util::input::Key;

/// The reserved owner of the kernel's own commands.
pub const KERNEL: PluginId = PluginId("kernel");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandId {
    pub owner: PluginId,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeySeq {
    Key(Key),
    Prefixed(char, char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyScope {
    Anywhere,
    Focused(FocusKey),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding {
    pub seq: KeySeq,
    pub scope: KeyScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Group {
    pub prefix: char,
    pub name: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub id: CommandId,
    pub label: &'static str,
    pub hint: &'static str,
    pub bindings: Vec<Binding>,
    pub palette: bool,
    pub group: Option<Group>,
    pub rank: u16,
}

/// The which-key data for one pending prefix: the group and its bindings,
/// ordered by `(rank, registration)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrefixGroupView {
    pub key: char,
    pub name: &'static str,
    pub bindings: Vec<(char, &'static str, CommandId)>,
}

/// Whether a binding scope is active given the optional focus.
pub fn scope_matches(scope: KeyScope, focus: Option<FocusKey>) -> bool {
    match scope {
        KeyScope::Anywhere => true,
        KeyScope::Focused(target) => focus == Some(target),
    }
}

/// The kernel-owned commands (section 4.8). The kernel contributes these to the
/// command registry itself; no plugin declares them.
pub fn kernel_commands() -> Vec<CommandSpec> {
    fn anywhere(seq: KeySeq) -> Binding {
        Binding {
            seq,
            scope: KeyScope::Anywhere,
        }
    }
    fn spec(
        name: &'static str,
        label: &'static str,
        hint: &'static str,
        bindings: Vec<Binding>,
        palette: bool,
        rank: u16,
    ) -> CommandSpec {
        CommandSpec {
            id: CommandId {
                owner: KERNEL,
                name,
            },
            label,
            hint,
            bindings,
            palette,
            group: None,
            rank,
        }
    }

    vec![
        spec("quit", "Quit", "q", Vec::new(), true, 390),
        spec(
            "pane.next",
            "Switch pane",
            "Tab",
            vec![anywhere(KeySeq::Key(Key::Tab))],
            true,
            320,
        ),
        spec(
            "pane.previous",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::BackTab))],
            false,
            0,
        ),
        spec(
            "nav.top",
            "Jump to top",
            "g",
            vec![
                anywhere(KeySeq::Key(Key::Char('g'))),
                anywhere(KeySeq::Key(Key::Home)),
            ],
            true,
            360,
        ),
        spec(
            "nav.bottom",
            "Jump to bottom",
            "G",
            vec![
                anywhere(KeySeq::Key(Key::Char('G'))),
                anywhere(KeySeq::Key(Key::End)),
            ],
            true,
            370,
        ),
        spec(
            "nav.page_up",
            "",
            "",
            vec![
                anywhere(KeySeq::Key(Key::PageUp)),
                anywhere(KeySeq::Key(Key::CtrlU)),
            ],
            false,
            0,
        ),
        spec(
            "nav.page_down",
            "",
            "",
            vec![
                anywhere(KeySeq::Key(Key::PageDown)),
                anywhere(KeySeq::Key(Key::CtrlD)),
            ],
            false,
            0,
        ),
        spec(
            "search.next",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::Char('n')))],
            false,
            0,
        ),
        spec(
            "search.previous",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::Char('N')))],
            false,
            0,
        ),
    ]
}

/// The section 4.8 registry invariants, checked over the in-scope bindings.
///
/// A binding is in scope when it is [`KeyScope::Anywhere`] or its focus matches
/// `focus`.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub fn validate(commands: &[CommandSpec], focus: Option<FocusKey>) -> Result<(), String> {
    let mut seen: Vec<KeySeq> = Vec::new();
    let mut chars: Vec<char> = Vec::new();
    let mut prefixes: Vec<char> = Vec::new();

    for spec in commands {
        for binding in &spec.bindings {
            if !scope_matches(binding.scope, focus) {
                continue;
            }
            if seen.contains(&binding.seq) {
                return Err(format!(
                    "{}:{} binds {:?} twice",
                    spec.id.owner.0, spec.id.name, binding.seq
                ));
            }
            seen.push(binding.seq);
            match binding.seq {
                KeySeq::Key(Key::Char(character)) => chars.push(character),
                KeySeq::Key(_) => {}
                KeySeq::Prefixed(prefix, _) => {
                    if spec.group.is_none() {
                        return Err(format!(
                            "{}:{} has a prefixed binding without a group",
                            spec.id.owner.0, spec.id.name
                        ));
                    }
                    prefixes.push(prefix);
                }
            }
        }
    }

    for prefix in &prefixes {
        if chars.contains(prefix) {
            return Err(format!(
                "character {prefix:?} is both a key binding and a prefix"
            ));
        }
    }

    Ok(())
}
