//! The keymap: the pending prefix and the binding lookups key dispatch uses.

#![allow(dead_code)]
use crate::kernel::command::{Binding, CommandId, CommandSpec, Group, KeySeq, scope_matches};
use crate::kernel::plugin::FocusKey;
use crate::util::input::Key;

/// The pending fold prefix. `pending` is `Some(c)` exactly while a
/// `Prefixed(c, _)` binding is armed and the which-key band is open.
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    pub pending: Option<char>,
}

fn in_scope(spec: &CommandSpec, focus: Option<FocusKey>) -> impl Iterator<Item = &Binding> {
    spec.bindings
        .iter()
        .filter(move |binding| scope_matches(binding.scope, focus))
}

/// The first in-scope command bound to `key` anywhere.
pub fn command_for_key(
    commands: &[CommandSpec],
    key: Key,
    focus: Option<FocusKey>,
) -> Option<CommandId> {
    commands.iter().find_map(|spec| {
        in_scope(spec, focus)
            .any(|binding| binding.seq == KeySeq::Key(key))
            .then_some(spec.id)
    })
}

/// Whether some in-scope `Prefixed(prefix, _)` binding is armed.
pub fn has_prefix(commands: &[CommandSpec], prefix: char, focus: Option<FocusKey>) -> bool {
    commands.iter().any(|spec| {
        in_scope(spec, focus)
            .any(|binding| matches!(binding.seq, KeySeq::Prefixed(p, _) if p == prefix))
    })
}

/// The command an in-scope `Prefixed(prefix, key)` binding runs.
pub fn prefixed_command(
    commands: &[CommandSpec],
    prefix: char,
    key: char,
    focus: Option<FocusKey>,
) -> Option<CommandId> {
    commands.iter().find_map(|spec| {
        in_scope(spec, focus)
            .any(|binding| binding.seq == KeySeq::Prefixed(prefix, key))
            .then_some(spec.id)
    })
}

/// One which-key group binding.
#[derive(Clone, Copy, Debug)]
pub struct GroupBinding {
    pub key: char,
    pub label: &'static str,
    pub hint: &'static str,
    pub id: CommandId,
}

/// The in-scope commands whose binding is `Prefixed(prefix, _)`, grouped and
/// ordered by rank.
pub fn prefix_group(
    commands: &[CommandSpec],
    prefix: char,
    focus: Option<FocusKey>,
) -> Option<(Group, Vec<GroupBinding>)> {
    let mut specs: Vec<&CommandSpec> = commands
        .iter()
        .filter(|spec| {
            in_scope(spec, focus)
                .any(|binding| matches!(binding.seq, KeySeq::Prefixed(p, _) if p == prefix))
        })
        .collect();
    specs.sort_by_key(|spec| spec.rank);

    let mut group: Option<Group> = None;
    let mut bindings = Vec::new();
    for spec in specs {
        let Some(spec_group) = spec.group else {
            continue;
        };
        group.get_or_insert(spec_group);
        let Some(key) = in_scope(spec, focus).find_map(|binding| match binding.seq {
            KeySeq::Prefixed(_, key) => Some(key),
            KeySeq::Key(_) => None,
        }) else {
            continue;
        };
        bindings.push(GroupBinding {
            key,
            label: spec.label,
            hint: spec.hint,
            id: spec.id,
        });
    }
    group.map(|group| (group, bindings))
}
