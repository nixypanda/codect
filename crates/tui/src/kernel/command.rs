//! The command vocabulary: identifiers, bindings, and the registry invariants.
//!
//! The registry data itself (`raw_commands` and its helpers) is built by the
//! `Legacy` plugin in this phase; this module holds only the types and the
//! section 4.8 validation. `Owner` is the temporary stand-in for `PluginId`
//! that tasks 4.2+ replace per plugin.

use crate::kernel::plugin::FocusKey;
use crate::util::input::Key;

/// The temporary owner of a command: the plugin id it will become.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Owner {
    Kernel,
    Show,
    Diff,
    Tree,
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandId {
    pub owner: Owner,
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
                    owner_name(spec.id.owner),
                    spec.id.name,
                    binding.seq
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
                            owner_name(spec.id.owner),
                            spec.id.name
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

#[cfg_attr(not(debug_assertions), allow(dead_code))]
fn owner_name(owner: Owner) -> &'static str {
    match owner {
        Owner::Kernel => "kernel",
        Owner::Show => "show",
        Owner::Diff => "diff",
        Owner::Tree => "tree",
        Owner::Overlay => "overlay",
    }
}
