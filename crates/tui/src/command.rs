//! The command registry: one table drives key dispatch, the command palette,
//! and the which-key band.
//!
//! The types live in [`crate::kernel::command`]; this module builds the table
//! and resolves a command name to its temporary [`Action`]. In this phase the
//! registry belongs to the `Legacy` plugin; tasks 4.2+ split it per plugin.

#![allow(dead_code)]
#[allow(unused_imports)]
pub(crate) use crate::kernel::command::{
    Binding, CommandId, CommandSpec, Group, KeyScope, KeySeq, Owner, validate,
};

use crate::action::{Action, CommitsAction, GlobalAction, PrefixGroup, RangeAction, ShowAction};
use crate::kernel::plugin::FocusKey;
use crate::page::{DiffViewState, Loaded};
use crate::plugin::SOURCE_FOCUS;
use crate::util::input::Key;

const FOLD: Group = Group {
    prefix: 'z',
    name: "fold",
};

fn anywhere(seq: KeySeq) -> Binding {
    Binding {
        seq,
        scope: KeyScope::Anywhere,
    }
}

fn focused(seq: KeySeq, focus: FocusKey) -> Binding {
    Binding {
        seq,
        scope: KeyScope::Focused(focus),
    }
}

#[allow(clippy::too_many_arguments)]
fn spec(
    owner: Owner,
    name: &'static str,
    label: &'static str,
    hint: &'static str,
    bindings: Vec<Binding>,
    palette: bool,
    group: Option<Group>,
    rank: u16,
) -> CommandSpec {
    CommandSpec {
        id: CommandId { owner, name },
        label,
        hint,
        bindings,
        palette,
        group,
        rank,
    }
}

/// The whole registry for the session `loaded` belongs to.
fn raw_commands(loaded: &Loaded) -> Vec<CommandSpec> {
    let mut commands = vec![
        spec(
            Owner::Kernel,
            "quit",
            "Quit",
            "q",
            Vec::new(),
            true,
            None,
            390,
        ),
        spec(
            Owner::Kernel,
            "pane.next",
            "Switch pane",
            "Tab",
            vec![anywhere(KeySeq::Key(Key::Tab))],
            true,
            None,
            320,
        ),
        spec(
            Owner::Kernel,
            "pane.previous",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::BackTab))],
            false,
            None,
            0,
        ),
        spec(
            Owner::Kernel,
            "nav.top",
            "Jump to top",
            "g",
            vec![
                anywhere(KeySeq::Key(Key::Char('g'))),
                anywhere(KeySeq::Key(Key::Home)),
            ],
            true,
            None,
            360,
        ),
        spec(
            Owner::Kernel,
            "nav.bottom",
            "Jump to bottom",
            "G",
            vec![
                anywhere(KeySeq::Key(Key::Char('G'))),
                anywhere(KeySeq::Key(Key::End)),
            ],
            true,
            None,
            370,
        ),
        spec(
            Owner::Kernel,
            "nav.page_up",
            "",
            "",
            vec![
                anywhere(KeySeq::Key(Key::PageUp)),
                anywhere(KeySeq::Key(Key::CtrlU)),
            ],
            false,
            None,
            0,
        ),
        spec(
            Owner::Kernel,
            "nav.page_down",
            "",
            "",
            vec![
                anywhere(KeySeq::Key(Key::PageDown)),
                anywhere(KeySeq::Key(Key::CtrlD)),
            ],
            false,
            None,
            0,
        ),
        spec(
            Owner::Kernel,
            "search.next",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::Char('n')))],
            false,
            None,
            0,
        ),
        spec(
            Owner::Kernel,
            "search.previous",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::Char('N')))],
            false,
            None,
            0,
        ),
        spec(
            Owner::Overlay,
            "modes.open",
            "Switch types / signatures / tests",
            "m",
            vec![anywhere(KeySeq::Key(Key::Char('m')))],
            true,
            None,
            100,
        ),
        spec(
            Owner::Overlay,
            "scope.open",
            "Change scope",
            "s",
            vec![anywhere(KeySeq::Key(Key::Char('s')))],
            true,
            None,
            110,
        ),
        spec(
            Owner::Overlay,
            "finder.open",
            "Find file",
            "Ctrl-F",
            vec![anywhere(KeySeq::Key(Key::CtrlF))],
            true,
            None,
            300,
        ),
        spec(
            Owner::Overlay,
            "search.open",
            "Search in view",
            "/",
            vec![anywhere(KeySeq::Key(Key::Char('/')))],
            true,
            None,
            310,
        ),
        spec(
            Owner::Overlay,
            "help.toggle",
            "Help",
            "?",
            vec![anywhere(KeySeq::Key(Key::Char('?')))],
            true,
            None,
            380,
        ),
        spec(
            Owner::Overlay,
            "palette.open",
            "",
            "",
            vec![anywhere(KeySeq::Key(Key::CtrlP))],
            false,
            None,
            0,
        ),
    ];

    // The tree plugin owns its three commands; the legacy palette and which-key
    // read them through this shared registry until task 4.7 removes the bridge.
    commands.extend(crate::tree::commands());

    match loaded {
        Loaded::Show(_) => {
            commands.push(spec(
                Owner::Show,
                "revision.edit",
                "Edit revision",
                "r",
                vec![anywhere(KeySeq::Key(Key::Char('r')))],
                true,
                None,
                200,
            ));
            for (name, label, hint, second, rank) in [
                ("fold.toggle", "Toggle fold", "za", 'a', 210),
                ("fold.open", "Open fold", "zo", 'o', 211),
                ("fold.close", "Close fold", "zc", 'c', 212),
                (
                    "fold.open_recursive",
                    "Open fold recursively",
                    "zO",
                    'O',
                    213,
                ),
                (
                    "fold.close_recursive",
                    "Close fold recursively",
                    "zC",
                    'C',
                    214,
                ),
                ("fold.open_all", "Open all folds", "zR", 'R', 215),
                ("fold.close_all", "Close all folds", "zM", 'M', 216),
            ] {
                commands.push(spec(
                    Owner::Show,
                    name,
                    label,
                    hint,
                    vec![focused(KeySeq::Prefixed('z', second), SOURCE_FOCUS)],
                    true,
                    Some(FOLD),
                    rank,
                ));
            }
        }
        Loaded::Diff(diff) => {
            let range = matches!(diff.view, DiffViewState::Range(_));
            let commits = matches!(diff.view, DiffViewState::Commits(_));
            let switchable = range
                && !engine::is_snapshot_spec(&diff.base)
                && !engine::is_snapshot_spec(&diff.target)
                && !diff.merge_base;

            commands.push(spec(
                Owner::Diff,
                "base.edit",
                "Edit base revision",
                "b",
                vec![anywhere(KeySeq::Key(Key::Char('b')))],
                range,
                None,
                200,
            ));
            commands.push(spec(
                Owner::Diff,
                "target.edit",
                "Edit target revision",
                "t",
                vec![anywhere(KeySeq::Key(Key::Char('t')))],
                range,
                None,
                201,
            ));
            commands.push(spec(
                Owner::Diff,
                "view.commits",
                "Switch to commits view",
                "",
                Vec::new(),
                switchable,
                None,
                202,
            ));
            commands.push(spec(
                Owner::Diff,
                "view.range",
                "Switch to range view",
                "",
                Vec::new(),
                commits,
                None,
                200,
            ));
        }
    }

    commands
}

/// The registry for `loaded`, validated in debug builds.
pub(crate) fn commands(loaded: &Loaded, focus: Option<FocusKey>) -> Vec<CommandSpec> {
    let specs = raw_commands(loaded);
    #[cfg(not(debug_assertions))]
    let _ = focus;
    #[cfg(debug_assertions)]
    if let Err(message) = validate(&specs, focus) {
        panic!("command registry validation failed: {message}");
    }
    specs
}

/// The temporary action a command name dispatches.
pub(crate) fn action_for(name: &str) -> Option<Action> {
    Some(match name {
        "quit" => Action::Global(GlobalAction::Quit),
        "pane.next" => Action::Global(GlobalAction::NextPane),
        "pane.previous" => Action::Global(GlobalAction::PreviousPane),
        "nav.top" => Action::Global(GlobalAction::Top),
        "nav.bottom" => Action::Global(GlobalAction::Bottom),
        "nav.page_up" => Action::Global(GlobalAction::PageUp),
        "nav.page_down" => Action::Global(GlobalAction::PageDown),
        "search.next" => Action::Global(GlobalAction::NextMatch),
        "search.previous" => Action::Global(GlobalAction::PreviousMatch),
        "wider" => Action::Global(GlobalAction::TreeWider),
        "narrower" => Action::Global(GlobalAction::TreeNarrower),
        "reset" => Action::Global(GlobalAction::TreeReset),
        "modes.open" => Action::Global(GlobalAction::Mode),
        "scope.open" => Action::Global(GlobalAction::Scope),
        "finder.open" => Action::Global(GlobalAction::Finder),
        "search.open" => Action::Global(GlobalAction::Search),
        "help.toggle" => Action::Global(GlobalAction::Help),
        "palette.open" => Action::Global(GlobalAction::Palette),
        "revision.edit" => Action::Show(ShowAction::EditRevision),
        "fold.toggle" => Action::Show(ShowAction::ToggleFold),
        "fold.open" => Action::Show(ShowAction::OpenFold),
        "fold.close" => Action::Show(ShowAction::CloseFold),
        "fold.open_recursive" => Action::Show(ShowAction::OpenRecursive),
        "fold.close_recursive" => Action::Show(ShowAction::CloseRecursive),
        "fold.open_all" => Action::Show(ShowAction::OpenAll),
        "fold.close_all" => Action::Show(ShowAction::CloseAll),
        "base.edit" => Action::Range(RangeAction::EditBase),
        "target.edit" => Action::Range(RangeAction::EditTarget),
        "view.commits" => Action::Range(RangeAction::SwitchToCommits),
        "view.range" => Action::Commits(CommitsAction::SwitchToRange),
        _ => return None,
    })
}

/// The first `Anywhere` command bound to `key`, matching today's
/// `command_for`.
pub(crate) fn action_for_key(loaded: &Loaded, focus: Option<FocusKey>, key: Key) -> Option<Action> {
    commands(loaded, focus).into_iter().find_map(|spec| {
        spec.bindings
            .iter()
            .any(|binding| binding.scope == KeyScope::Anywhere && binding.seq == KeySeq::Key(key))
            .then(|| action_for(spec.id.name))
            .flatten()
    })
}

/// The prefix `key` parks, if any `Prefixed` binding is in scope.
pub(crate) fn pending_prefix(loaded: &Loaded, focus: Option<FocusKey>, key: Key) -> Option<char> {
    let Key::Char(character) = key else {
        return None;
    };
    let groups = prefix_groups_in_scope(loaded, focus);
    groups
        .iter()
        .any(|group| group.key == character)
        .then_some(character)
}

/// Today's `Loaded::palette_entries`: the `palette == true` subset, ordered by
/// `(rank, registration order)`.
pub(crate) fn palette_entries(loaded: &Loaded) -> Vec<(Action, &'static str, &'static str)> {
    let mut specs: Vec<CommandSpec> = raw_commands(loaded)
        .into_iter()
        .filter(|spec| spec.palette)
        .collect();
    specs.sort_by_key(|spec| spec.rank);
    specs
        .into_iter()
        .filter_map(|spec| Some((action_for(spec.id.name)?, spec.label, spec.hint)))
        .collect()
}

/// Today's `Loaded::prefix_groups`: the grouped `Prefixed` subset for the
/// session, ordered by rank.
pub(crate) fn prefix_groups(loaded: &Loaded) -> Vec<PrefixGroup> {
    collect_prefix_groups(loaded, |_| true)
}

/// The grouped `Prefixed` subset whose bindings are in scope for `focus`.
pub(crate) fn prefix_groups_in_scope(loaded: &Loaded, focus: Option<FocusKey>) -> Vec<PrefixGroup> {
    collect_prefix_groups(loaded, |scope| scope_matches(scope, focus))
}

fn collect_prefix_groups(loaded: &Loaded, in_scope: impl Fn(KeyScope) -> bool) -> Vec<PrefixGroup> {
    let mut specs: Vec<CommandSpec> = raw_commands(loaded)
        .into_iter()
        .filter(|spec| {
            spec.bindings.iter().any(|binding| {
                matches!(binding.seq, KeySeq::Prefixed(..)) && in_scope(binding.scope)
            })
        })
        .collect();
    specs.sort_by_key(|spec| spec.rank);

    let mut groups: Vec<PrefixGroup> = Vec::new();
    for spec in specs {
        let Some(group) = spec.group else {
            continue;
        };
        let Some((prefix, key)) = spec.bindings.iter().find_map(|binding| match binding.seq {
            KeySeq::Prefixed(prefix, key) => Some((prefix, key)),
            KeySeq::Key(_) => None,
        }) else {
            continue;
        };
        let Some(action) = action_for(spec.id.name) else {
            continue;
        };
        let binding = crate::action::Binding {
            key,
            action,
            label: spec.label,
            hint: spec.hint,
        };
        match groups.iter_mut().find(|existing| existing.key == prefix) {
            Some(existing) => existing.bindings.push(binding),
            None => groups.push(PrefixGroup {
                key: prefix,
                name: group.name,
                bindings: vec![binding],
            }),
        }
    }
    groups
}

fn scope_matches(scope: KeyScope, focus: Option<FocusKey>) -> bool {
    crate::kernel::command::scope_matches(scope, focus)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, TREE_FOCUS};
    use crate::test_support::{commits_app, diff_app, file_diff, folder_app};

    fn test_command(name: &'static str, bindings: Vec<Binding>) -> CommandSpec {
        spec(Owner::Kernel, name, "", "", bindings, false, None, 0)
    }

    fn bound(seq: KeySeq, scope: KeyScope) -> Binding {
        Binding { seq, scope }
    }

    #[test]
    fn validation_rejects_a_duplicate_key() {
        let both_anywhere = vec![
            test_command(
                "a",
                vec![bound(KeySeq::Key(Key::Char('x')), KeyScope::Anywhere)],
            ),
            test_command(
                "b",
                vec![bound(KeySeq::Key(Key::Char('x')), KeyScope::Anywhere)],
            ),
        ];
        assert!(validate(&both_anywhere, None).is_err());

        let boundary = vec![
            test_command(
                "a",
                vec![bound(KeySeq::Key(Key::Char('x')), KeyScope::Anywhere)],
            ),
            test_command(
                "b",
                vec![bound(
                    KeySeq::Key(Key::Char('x')),
                    KeyScope::Focused(TREE_FOCUS),
                )],
            ),
        ];
        assert!(validate(&boundary, Some(TREE_FOCUS)).is_err());
        assert!(validate(&boundary, Some(SOURCE_FOCUS)).is_ok());
        assert!(validate(&boundary, None).is_ok());
    }

    #[test]
    fn validation_rejects_a_key_that_is_also_a_prefix() {
        let conflict = vec![
            test_command(
                "a",
                vec![bound(KeySeq::Key(Key::Char('x')), KeyScope::Anywhere)],
            ),
            spec(
                Owner::Show,
                "fold",
                "",
                "",
                vec![bound(
                    KeySeq::Prefixed('x', 'a'),
                    KeyScope::Focused(SOURCE_FOCUS),
                )],
                true,
                Some(Group {
                    prefix: 'x',
                    name: "group",
                }),
                210,
            ),
        ];
        assert!(validate(&conflict, Some(SOURCE_FOCUS)).is_err());
        assert!(validate(&conflict, Some(TREE_FOCUS)).is_ok());
    }

    #[test]
    fn validation_rejects_a_prefixed_binding_without_a_group() {
        let missing = vec![spec(
            Owner::Show,
            "fold",
            "",
            "",
            vec![bound(
                KeySeq::Prefixed('z', 'a'),
                KeyScope::Focused(SOURCE_FOCUS),
            )],
            true,
            None,
            210,
        )];
        assert!(validate(&missing, Some(SOURCE_FOCUS)).is_err());
    }

    #[test]
    fn the_real_registry_validates_in_every_view_and_focus() {
        let show = folder_app();
        let range = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let commits = commits_app(3);

        // The plugin-set union: the tree plugin's commands plus the legacy
        // registry, exactly as the kernel collects them.
        for app in [&show, &range, &commits] {
            let specs = crate::kernel::app::collect_specs(app).commands;
            for focus in [
                Some(TREE_FOCUS),
                Some(SOURCE_FOCUS),
                Some(DIFF_BODY_FOCUS),
                Some(DIFF_BAND_FOCUS),
                None,
            ] {
                assert!(
                    validate(&specs, focus).is_ok(),
                    "focus {focus:?}: {:?}",
                    validate(&specs, focus)
                );
            }
        }

        let folds = |app: &crate::test_support::App| -> usize {
            crate::kernel::app::collect_specs(app)
                .commands
                .iter()
                .filter(|spec| spec.id.owner == Owner::Show && spec.id.name.starts_with("fold."))
                .count()
        };
        assert_eq!(folds(&show), 7);
        assert_eq!(folds(&range), 0);
        assert_eq!(folds(&commits), 0);
    }
}
