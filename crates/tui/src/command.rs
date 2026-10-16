//! The command registry: one table drives key dispatch, the command palette,
//! and the which-key band.
//!
//! This is the temporary, in-crate form of the vocabulary in the plugin plan's
//! section 4.6. `PluginId` and `FocusKey` do not exist yet, so [`Owner`] and the
//! existing [`Focus`] stand in for them, and [`CommandSpec::run`] carries the
//! resolved [`Action`] that the per-plugin effects of 4.x will eventually
//! replace.
//!
//! [`commands`] reproduces today's `command_for`, `palette_entries`, and
//! `fold_bindings` byte-for-byte. The kernel and explorer commands are emitted
//! in both sessions; the source and diff commands are emitted only in their own
//! session. [`validate`] enforces the section 4.8 registry invariants.

use crate::action::{Action, CommitsAction, GlobalAction, PrefixGroup, RangeAction, ShowAction};
use crate::app::App;
use crate::page::{DiffViewState, Loaded};
use crate::render::metrics::Focus;
use crate::util::input::Key;

/// The temporary owner of a command: the plugin id it will become.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Owner {
    Kernel,
    Show,
    Diff,
    Tree,
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandId {
    pub owner: Owner,
    pub name: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeySeq {
    Key(Key),
    Prefixed(char, char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum KeyScope {
    Anywhere,
    Focused(Focus),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Binding {
    pub seq: KeySeq,
    pub scope: KeyScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Group {
    pub prefix: char,
    pub name: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandSpec {
    pub id: CommandId,
    pub label: &'static str,
    pub hint: &'static str,
    pub bindings: Vec<Binding>,
    pub palette: bool,
    pub group: Option<Group>,
    pub rank: u16,
    /// Temporary: the effect this command dispatches, until 4.x moves `run` to
    /// the owning plugin.
    pub run: Action,
}

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

fn focused(seq: KeySeq, focus: Focus) -> Binding {
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
    run: Action,
) -> CommandSpec {
    CommandSpec {
        id: CommandId { owner, name },
        label,
        hint,
        bindings,
        palette,
        group,
        rank,
        run,
    }
}

/// The whole registry for the session `loaded` belongs to.
///
/// Registration order is stable but not behaviour: the palette and which-key
/// order by `rank`, and no two commands bind the same key sequence.
fn raw_commands(loaded: &Loaded) -> Vec<CommandSpec> {
    let mut commands = vec![
        // Kernel commands exist in both sessions.
        spec(
            Owner::Kernel,
            "quit",
            "Quit",
            "q",
            Vec::new(),
            true,
            None,
            390,
            Action::Global(GlobalAction::Quit),
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
            Action::Global(GlobalAction::NextPane),
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
            Action::Global(GlobalAction::PreviousPane),
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
            Action::Global(GlobalAction::Top),
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
            Action::Global(GlobalAction::Bottom),
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
            Action::Global(GlobalAction::PageUp),
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
            Action::Global(GlobalAction::PageDown),
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
            Action::Global(GlobalAction::NextMatch),
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
            Action::Global(GlobalAction::PreviousMatch),
        ),
        // The explorer's resize commands exist in both sessions.
        spec(
            Owner::Tree,
            "wider",
            "Widen file tree",
            "]",
            vec![anywhere(KeySeq::Key(Key::Char(']')))],
            true,
            None,
            330,
            Action::Global(GlobalAction::TreeWider),
        ),
        spec(
            Owner::Tree,
            "narrower",
            "Narrow file tree",
            "[",
            vec![anywhere(KeySeq::Key(Key::Char('[')))],
            true,
            None,
            340,
            Action::Global(GlobalAction::TreeNarrower),
        ),
        spec(
            Owner::Tree,
            "reset",
            "Reset file tree",
            "\\",
            vec![anywhere(KeySeq::Key(Key::Char('\\')))],
            true,
            None,
            350,
            Action::Global(GlobalAction::TreeReset),
        ),
        // The overlay roots exist in both sessions.
        spec(
            Owner::Overlay,
            "modes.open",
            "Switch types / signatures / tests",
            "m",
            vec![anywhere(KeySeq::Key(Key::Char('m')))],
            true,
            None,
            100,
            Action::Global(GlobalAction::Mode),
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
            Action::Global(GlobalAction::Scope),
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
            Action::Global(GlobalAction::Finder),
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
            Action::Global(GlobalAction::Search),
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
            Action::Global(GlobalAction::Help),
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
            Action::Global(GlobalAction::Palette),
        ),
    ];

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
                Action::Show(ShowAction::EditRevision),
            ));
            for (name, label, hint, second, rank, action) in [
                (
                    "fold.toggle",
                    "Toggle fold",
                    "za",
                    'a',
                    210,
                    ShowAction::ToggleFold,
                ),
                (
                    "fold.open",
                    "Open fold",
                    "zo",
                    'o',
                    211,
                    ShowAction::OpenFold,
                ),
                (
                    "fold.close",
                    "Close fold",
                    "zc",
                    'c',
                    212,
                    ShowAction::CloseFold,
                ),
                (
                    "fold.open_recursive",
                    "Open fold recursively",
                    "zO",
                    'O',
                    213,
                    ShowAction::OpenRecursive,
                ),
                (
                    "fold.close_recursive",
                    "Close fold recursively",
                    "zC",
                    'C',
                    214,
                    ShowAction::CloseRecursive,
                ),
                (
                    "fold.open_all",
                    "Open all folds",
                    "zR",
                    'R',
                    215,
                    ShowAction::OpenAll,
                ),
                (
                    "fold.close_all",
                    "Close all folds",
                    "zM",
                    'M',
                    216,
                    ShowAction::CloseAll,
                ),
            ] {
                commands.push(spec(
                    Owner::Show,
                    name,
                    label,
                    hint,
                    vec![focused(KeySeq::Prefixed('z', second), Focus::Content)],
                    true,
                    Some(FOLD),
                    rank,
                    Action::Show(action),
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
                Action::Range(RangeAction::EditBase),
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
                Action::Range(RangeAction::EditTarget),
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
                Action::Range(RangeAction::SwitchToCommits),
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
                Action::Commits(CommitsAction::SwitchToRange),
            ));
        }
    }

    commands
}

/// The registry for `app`, validated in debug builds.
pub(crate) fn commands(app: &App) -> Vec<CommandSpec> {
    let specs = raw_commands(&app.loaded);
    #[cfg(debug_assertions)]
    if let Err(message) = validate(&specs, Some(app.loaded.focus())) {
        panic!("command registry validation failed: {message}");
    }
    specs
}

/// The first `Anywhere` command bound to `key`, matching today's
/// `command_for`.
pub(crate) fn action_for_key(app: &App, key: Key) -> Option<Action> {
    commands(app).into_iter().find_map(|spec| {
        spec.bindings
            .iter()
            .any(|binding| binding.scope == KeyScope::Anywhere && binding.seq == KeySeq::Key(key))
            .then_some(spec.run)
    })
}

/// The prefix `key` parks, if any `Prefixed` binding is in scope.
pub(crate) fn pending_prefix(app: &App, key: Key) -> Option<char> {
    let Key::Char(character) = key else {
        return None;
    };
    let groups = prefix_groups_in_scope(&app.loaded, Some(app.loaded.focus()));
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
        .map(|spec| (spec.run, spec.label, spec.hint))
        .collect()
}

/// Today's `Loaded::prefix_groups`: the grouped `Prefixed` subset for the
/// session, ordered by rank.
pub(crate) fn prefix_groups(loaded: &Loaded) -> Vec<PrefixGroup> {
    collect_prefix_groups(loaded, |_| true)
}

/// The grouped `Prefixed` subset whose bindings are in scope for `focus`.
pub(crate) fn prefix_groups_in_scope(loaded: &Loaded, focus: Option<Focus>) -> Vec<PrefixGroup> {
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
        let binding = crate::action::Binding {
            key,
            action: spec.run,
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

/// Whether a binding scope is active given the optional focus.
fn scope_matches(scope: KeyScope, focus: Option<Focus>) -> bool {
    match scope {
        KeyScope::Anywhere => true,
        KeyScope::Focused(target) => focus == Some(target),
    }
}

/// The section 4.8 registry invariants, checked over the in-scope bindings.
///
/// A binding is in scope when it is [`KeyScope::Anywhere`] or its focus matches
/// `focus`.
pub(crate) fn validate(commands: &[CommandSpec], focus: Option<Focus>) -> Result<(), String> {
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

fn owner_name(owner: Owner) -> &'static str {
    match owner {
        Owner::Kernel => "kernel",
        Owner::Show => "show",
        Owner::Diff => "diff",
        Owner::Tree => "tree",
        Owner::Overlay => "overlay",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{commits_app, diff_app, file_diff, folder_app};

    fn test_command(name: &'static str, bindings: Vec<Binding>) -> CommandSpec {
        spec(
            Owner::Kernel,
            name,
            "",
            "",
            bindings,
            false,
            None,
            0,
            Action::Global(GlobalAction::Quit),
        )
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

        // An `Anywhere` binding collides with a `Focused` one only when the
        // focus matches.
        let boundary = vec![
            test_command(
                "a",
                vec![bound(KeySeq::Key(Key::Char('x')), KeyScope::Anywhere)],
            ),
            test_command(
                "b",
                vec![bound(
                    KeySeq::Key(Key::Char('x')),
                    KeyScope::Focused(Focus::Tree),
                )],
            ),
        ];
        assert!(validate(&boundary, Some(Focus::Tree)).is_err());
        assert!(validate(&boundary, Some(Focus::Content)).is_ok());
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
                    KeyScope::Focused(Focus::Content),
                )],
                true,
                Some(Group {
                    prefix: 'x',
                    name: "group",
                }),
                210,
                Action::Show(ShowAction::ToggleFold),
            ),
        ];
        assert!(validate(&conflict, Some(Focus::Content)).is_err());
        // Disjoint scopes: away from `Content` the prefix is out of scope.
        assert!(validate(&conflict, Some(Focus::Tree)).is_ok());
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
                KeyScope::Focused(Focus::Content),
            )],
            true,
            None,
            210,
            Action::Show(ShowAction::ToggleFold),
        )];
        assert!(validate(&missing, Some(Focus::Content)).is_err());
    }

    #[test]
    fn the_real_registry_validates_in_every_view_and_focus() {
        let show = folder_app();
        let range = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let commits = commits_app(3);

        for loaded in [&show.loaded, &range.loaded, &commits.loaded] {
            let specs = raw_commands(loaded);
            for focus in [
                Some(Focus::Tree),
                Some(Focus::Content),
                Some(Focus::Commits),
                None,
            ] {
                assert!(
                    validate(&specs, focus).is_ok(),
                    "focus {focus:?}: {:?}",
                    validate(&specs, focus)
                );
            }
        }

        let folds = |loaded: &Loaded| -> usize {
            raw_commands(loaded)
                .iter()
                .filter(|spec| spec.id.owner == Owner::Show && spec.id.name.starts_with("fold."))
                .count()
        };
        assert_eq!(folds(&show.loaded), 7);
        assert_eq!(folds(&range.loaded), 0);
        assert_eq!(folds(&commits.loaded), 0);
    }
}
