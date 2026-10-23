#[cfg(feature = "diff")]
use crate::plugin::{
    COMMITS_PANE, DIFF, DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, NEW_PANE, OLD_PANE, TREE,
};
use crate::plugin::{
    HELP_PANE, MODES_PANE, PROMPT_PANE, SCOPE_PANE, SEARCH_PANE, SOURCE, SOURCE_FOCUS, SOURCE_PANE,
    TREE_FOCUS, TREE_PANE, WHICH_KEY_PANE,
};
use crate::plugins::PluginsCmd;
#[cfg(feature = "diff")]
use crate::route::{DiffRequest, DiffView};
use crate::route::{LoadRequest, ShowRequest};
use crate::scope_chooser::Msg as ScopeMsg;
use crate::source::Msg as SourceMsg;
use crate::source::{ShowHighlights, VisibleRow};
use crate::test_support::{
    ATTRIBUTED_SOURCE, App, NESTED_SOURCE, area_set, assert_view_consistent,
    attributed_child_outline, base_app, buffer_row, buffer_text, cell_x_of, click, command_spec,
    declaration_less, dropped_container_outline, focus_content, folder_app, has_palette_entry,
    item, multi_method_outline, nested_outline, palette_entries, press, render, row_with,
    scope_msg, scroll, select_file, selected, show_app, show_app_mode, show_ref, show_request,
    source_msg, source_outline, source_outline_with, test_function_outline, tests_filter_outline,
    two_files, two_function_outline, with_cx,
};
#[cfg(feature = "diff")]
use crate::test_support::{
    commit_search, commit_steps, commits_app, commits_ref, commits_request, diff_app,
    diff_focus_of, diff_msg, diff_ref, diff_request, file_diff, merge_base_range_app, panes,
    snapshot_range_app, tail_palette_entries,
};
#[cfg(feature = "diff")]
use crate::workspace::BadgeTone;
use crate::workspace::{Search, SearchMatch, SearchPane, collect_matches};
use base::{
    Area, ProjectionMode, RepoPath, Selection, SelectionGroup, SourceOutline, SupportedPath,
    assemble_outline,
};
use ratatui::style::Color;
use tui_core::app::{Diagnostic, Msg, settle, update};
#[cfg(feature = "diff")]
use tui_core::focus::{focus_default, focus_ring};
use tui_core::render::icons::IconStyle;
use tui_core::render::theme::Theme;
use tui_core::widgets::text_input::TextInput;
#[cfg(feature = "diff")]
use tui_diff::Msg as DiffMsg;

use crate::tree::RowKind;
use engine::EngineError;
use tui_core::command::CommandId;
use tui_core::plugin::{Event, Input, Out, Plugin, PluginSet, QueryChange, Request, Size};
#[cfg(feature = "diff")]
use tui_core::plugin::{FocusKey, NavigatorResize};
use tui_core::render::icons::Icons;
use tui_core::util::input::Key;

const TEST_SOURCE: &str = "#[test]\nfn decode_source() {\n    let hidden = 42;\n}\n";

const TREE_DEFAULT_PERCENT: u16 = 30;
const TREE_MIN_PERCENT: u16 = 15;
const TREE_MAX_PERCENT: u16 = 60;
const TREE_STEP: u16 = 5;
const DIAGNOSTIC_TICKS: u8 = 40;

fn apply_request(app: &mut App, request: Request, cmds: &mut Vec<PluginsCmd>) {
    let specs = tui_core::app::collect_specs(app);
    tui_core::dispatch::apply_request(app, request, cmds, 0, &specs);
}

fn apply_command(
    app: &mut App,
    owner: tui_core::plugin::PluginId,
    name: &'static str,
    cmds: &mut Vec<PluginsCmd>,
) {
    apply_request(app, Request::RunCommand(CommandId { owner, name }), cmds);
}

#[cfg(feature = "diff")]
fn coalesce_commit_loads(app: &mut App, cmds: &mut Vec<PluginsCmd>) {
    let effects = crate::test_support::after_batch_effects(app, cmds);
    cmds.extend(effects);
}

#[cfg(feature = "diff")]
fn command_for(key: Key, app: &App) -> Option<CommandId> {
    with_cx(app, |cx| {
        tui_core::keymap::command_for_key(cx.commands(), key, app.focus)
    })
}

fn tree_viewport_height(app: &App) -> usize {
    let specs = tui_core::app::collect_specs(app);
    let overlay = app.kernel.overlay;
    let busy = app.plugins.busy();
    let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);
    cx.layout_if_focused(TREE_FOCUS)
        .slots
        .iter()
        .find(|slot| slot.pane == TREE_PANE)
        .map_or(0, |slot| slot.geometry.inner.height as usize)
}

#[cfg(feature = "diff")]
fn body_is_visible(app: &App) -> bool {
    let specs = tui_core::app::collect_specs(app);
    let overlay = app.kernel.overlay;
    let busy = app.plugins.busy();
    let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);
    cx.body_visible()
}

#[test]
fn a_loaded_result_installs_files_and_preserves_the_selection() {
    let app = two_files();
    let (next, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![source_outline("a.rs", "A\n"), source_outline("b.rs", "B\n")].into()),
        }),
        &app,
    );

    assert_eq!(next.workspace.query.mode, ProjectionMode::Signatures);
    assert_eq!(selected(&next).as_deref(), Some("a.rs"));
    assert_eq!(
        show_ref(&next).active_source(next.workspace.selected.as_ref()),
        Some("A\n")
    );
}

#[test]
fn a_reload_that_drops_the_selected_file_picks_the_nearest_visible_one() {
    let mut app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("b.rs", "b\n"),
        source_outline("c.rs", "c\n"),
    ]);
    app.plugins.tree.cursor = app
        .plugins
        .tree
        .row_of_file(&RepoPath::new("b.rs").unwrap())
        .unwrap();
    app.workspace.selected = Some(RepoPath::new("b.rs").unwrap());

    let (next, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: show_request(),
            result: Ok(vec![source_outline("a.rs", "a\n"), source_outline("c.rs", "c\n")].into()),
        }),
        &app,
    );
    assert_eq!(selected(&next).as_deref(), Some("c.rs"));
}

#[test]
fn a_reload_retains_the_selection_and_resets_the_show_body() {
    let mut app = show_app(vec![source_outline("a.rs", "one\ntwo\nthree\n")]);
    let show = &mut app.plugins.source;
    show.body.scroll = 2;
    show.body.hscroll = 1;
    show.body.cursor_line = 3;

    let (next, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: show_request(),
            result: Ok(vec![source_outline("a.rs", "one\ntwo\nthree\n")].into()),
        }),
        &app,
    );

    assert_eq!(selected(&next).as_deref(), Some("a.rs"));
    let show = show_ref(&next);
    assert_eq!(show.body.scroll, 0, "a reload resets the scroll");
    assert_eq!(show.body.hscroll, 0);
    assert_eq!(show.body.cursor_line, 1);
}

#[cfg(feature = "diff")]
#[test]
fn a_reload_retains_the_selection_and_resets_the_diff_body() {
    let mut app = diff_app(vec![file_diff(
        "a.rs",
        Some("1\n2\n3\n4\n5\n6\n"),
        Some("1\n2\n3\n4\n5\nX\n"),
    )]);
    let diff = &mut app.plugins.diff;
    diff.set_body_scroll(3);

    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request: diff_request(),
            result: Ok(vec![file_diff(
                "a.rs",
                Some("1\n2\n3\n4\n5\n6\n"),
                Some("1\n2\n3\n4\n5\nX\n"),
            )]
            .into()),
        }),
        &app,
    );

    assert_eq!(selected(&next).as_deref(), Some("a.rs"));
    assert_eq!(
        diff_ref(&next).body_scroll(),
        0,
        "a reload resets the scroll"
    );
}

#[test]
fn a_reload_that_drops_the_selection_falls_back_to_the_last_file() {
    let app = press(&two_files(), &[Key::Char('j')]);
    assert_eq!(selected(&app).as_deref(), Some("b.rs"));

    let (next, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: show_request(),
            result: Ok(vec![source_outline("a.rs", "a\n")].into()),
        }),
        &app,
    );

    assert_eq!(
        selected(&next).as_deref(),
        Some("a.rs"),
        "with only earlier files, the last file is chosen"
    );
}

#[test]
fn an_empty_reload_clears_the_selection_and_the_tree_cursor() {
    let app = two_files();
    let (next, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: show_request(),
            result: Ok(Vec::new().into()),
        }),
        &app,
    );

    assert!(next.workspace.selected.is_none());
    assert_eq!(next.plugins.tree.cursor, 0);
}

#[cfg(feature = "diff")]
#[test]
fn a_diff_reload_applies_the_fallback_selection_within_the_update() {
    let app = diff_app(vec![
        file_diff("a.rs", Some("a\n"), Some("A\n")),
        file_diff("b.rs", Some("b\n"), Some("B\n")),
    ]);
    assert_eq!(selected(&app).as_deref(), Some("a.rs"));

    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request: diff_request(),
            result: Ok(vec![file_diff("b.rs", Some("b\n"), Some("B\n"))].into()),
        }),
        &app,
    );

    assert_eq!(selected(&next).as_deref(), Some("b.rs"));
    assert_eq!(
        next.plugins.tree.cursor, 0,
        "the fallback is revealed in the tree"
    );
}

#[test]
fn a_show_file_set_has_no_badges() {
    let app = two_files();
    assert!(app.workspace.files.badges.is_empty());
}

#[cfg(feature = "diff")]
#[test]
fn a_diff_file_set_badges_added_modified_and_deleted() {
    let app = diff_app(vec![
        file_diff("a.rs", None, Some("a\n")),
        file_diff("b.rs", Some("b\n"), Some("B\n")),
        file_diff("c.rs", Some("c\n"), None),
    ]);
    let badges = &app.workspace.files.badges;
    assert_eq!(
        badges.get(&RepoPath::new("a.rs").unwrap()),
        Some(&BadgeTone::Added)
    );
    assert_eq!(
        badges.get(&RepoPath::new("b.rs").unwrap()),
        Some(&BadgeTone::Modified)
    );
    assert_eq!(
        badges.get(&RepoPath::new("c.rs").unwrap()),
        Some(&BadgeTone::Deleted)
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("a.rs A"), "added badge missing: {text}");
    assert!(text.contains("b.rs M"), "modified badge missing: {text}");
    assert!(text.contains("c.rs D"), "deleted badge missing: {text}");
}

#[test]
fn an_empty_show_session_renders_the_no_files_empty_text() {
    let app = show_app(Vec::new());
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("No files"), "{text}");
    assert!(text.contains("No source files in this scope."), "{text}");
}

#[cfg(feature = "diff")]
#[test]
fn an_empty_diff_session_renders_the_no_changes_empty_text() {
    let app = diff_app(Vec::new());
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("No changes"), "{text}");
    assert!(text.contains("Body-only edits are omitted."), "{text}");
}

#[test]
fn a_busy_empty_show_renders_the_loading_state() {
    let app = base_app(LoadRequest::Show {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
    });
    assert!(app.is_busy());
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Loading"), "{text}");
    assert!(text.contains("Projecting files…"), "{text}");
}

#[cfg(feature = "diff")]
#[test]
fn a_busy_empty_diff_renders_the_loading_state() {
    let app = base_app(LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Range,
        merge_base: false,
    });
    assert!(app.is_busy());
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Loading"), "{text}");
    assert!(text.contains("Comparing projections…"), "{text}");
}

#[test]
fn a_failed_reload_keeps_the_last_model_and_shows_a_diagnostic() {
    let app = two_files();
    let error = EngineError::Selection(base::SelectionError::UnknownArea {
        name: "x".to_owned(),
    });
    let (next, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Err(Box::new(error)),
        }),
        &app,
    );

    assert_eq!(
        next.workspace.query.mode,
        ProjectionMode::Types,
        "mode must not change"
    );
    assert_eq!(show_ref(&next).files.len(), 2);
    assert!(next.diagnostic.is_some());
    assert!(next.workspace.selected.is_some());
}

#[test]
fn empty_projections_are_still_explorable_in_the_tree() {
    let app = show_app(vec![
        source_outline_with("empty.rs", "// comment only\n", ""),
        source_outline("full.rs", "x\n"),
    ]);
    let labels: Vec<&str> = app
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["empty.rs", "full.rs"]);
    assert!(
        app.workspace
            .files
            .paths
            .contains(&RepoPath::new("empty.rs").unwrap()),
        "a file with an empty projection must still appear in the tree"
    );
}

#[test]
fn a_declaration_less_file_is_visible_and_renders_its_full_source() {
    let source = "// only a comment\nuse std::fmt;\n";
    let app = show_app(vec![
        declaration_less("comments.rs", source),
        source_outline("full.rs", "x\n"),
    ]);

    assert!(
        app.workspace
            .files
            .paths
            .contains(&RepoPath::new("comments.rs").unwrap()),
        "a declaration-less file must appear in the tree"
    );
    let show = show_ref(&app);
    assert_eq!(
        show.active_source(app.workspace.selected.as_ref()),
        Some(source)
    );
    assert_eq!(show.line_count(), 2, "no fold rows");
    assert!(
        show.visible_rows()
            .iter()
            .all(|row| matches!(row, VisibleRow::Source { .. })),
        "a declaration-less file has no folds: {:?}",
        show.visible_rows()
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("only a comment"), "{text}");
    assert!(text.contains("use std::fmt;"), "{text}");
}

#[cfg(feature = "diff")]
#[test]
fn a_diff_tree_contains_only_changed_paths() {
    let app = diff_app(vec![
        file_diff("src/a.rs", Some("a\n"), Some("A\n")),
        file_diff("src/b.rs", None, Some("b\n")),
    ]);
    let labels: Vec<&str> = app
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["src", "a.rs", "b.rs"]);
}

#[test]
fn tree_rows_follow_raw_path_order_and_are_nested() {
    let app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("src/lib.rs", "l\n"),
        source_outline("src/main.rs", "m\n"),
    ]);
    let labels: Vec<&str> = app
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
    assert_eq!(app.plugins.tree.rows[0].depth, 0);
    assert_eq!(app.plugins.tree.rows[1].depth, 0);
    assert_eq!(app.plugins.tree.rows[2].depth, 1);
}

#[test]
fn collapsing_a_directory_hides_its_children_and_expanding_restores_them() {
    let mut app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("src/lib.rs", "l\n"),
        source_outline("src/main.rs", "m\n"),
    ]);
    app.plugins.tree.cursor = 1;
    assert!(matches!(
        app.plugins.tree.current_row().unwrap().kind,
        RowKind::Directory { .. }
    ));

    let (collapsed, _) = update(Msg::Key(Key::Enter), &app);
    let labels: Vec<&str> = collapsed
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["a.rs", "src"]);
    assert!(matches!(
        collapsed.plugins.tree.rows[1].kind,
        RowKind::Directory {
            expanded: false,
            ..
        }
    ));

    let (expanded, _) = update(Msg::Key(Key::Right), &collapsed);
    let labels: Vec<&str> = expanded
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
}

#[test]
fn left_on_a_directory_row_folds_it() {
    let mut app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("src/lib.rs", "l\n"),
    ]);
    app.plugins.tree.cursor = 1;
    let (folded, _) = update(Msg::Key(Key::Left), &app);
    assert!(matches!(
        folded.plugins.tree.rows[1].kind,
        RowKind::Directory {
            expanded: false,
            ..
        }
    ));
}

#[cfg(feature = "diff")]
#[test]
fn empty_commit_history_installs_a_diff_and_focuses_commits() {
    let app = base_app(LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Commits,
        merge_base: false,
    });
    let (next, commands) = update(
        diff_msg(DiffMsg::HistoryLoaded {
            request: commits_request(),
            result: Ok((Vec::new().into(), Vec::new().into())),
        }),
        &app,
    );
    assert!(commands.is_empty());
    assert_eq!(next.workspace.query.mode, ProjectionMode::Types);
    assert!(commits_ref(&next).is_empty());
    assert!(next.workspace.files.paths.is_empty());
    assert!(next.workspace.selected.is_none());
    let (unchanged, commands) = update(Msg::Key(Key::Down), &next);
    assert!(commands.is_empty());
    assert_eq!(commits_ref(&unchanged).cursor(), 0);
}

#[cfg(feature = "diff")]
#[test]
fn commit_picker_renders_rows_and_selected_revision_labels() {
    let app = commits_app(12);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Commits"), "{text}");
    assert!(text.contains("Files"), "{text}");
    assert!(text.contains("Commit sub"), "{text}");
    assert!(text.contains("1/12"), "{text}");
    assert!(text.contains("0000000..0101010"), "{text}");

    let mut empty = commits_app(0);
    let text = buffer_text(&render(&empty, 100, 20));
    assert!(text.contains("No commits"), "{text}");
    let _ = &mut empty;
}

#[cfg(feature = "diff")]
#[test]
fn commit_picker_mouse_uses_drawn_geometry_and_loads_selected_step() {
    let mut app = commits_app(12);
    app.size = Size {
        width: 100,
        height: 20,
    };
    let (mut clicked, mut commands) = update(Msg::Mouse(click(4, 12)), &app);
    coalesce_commit_loads(&mut clicked, &mut commands);
    assert_eq!(diff_focus_of(&clicked), Some(DIFF_BAND_FOCUS));
    assert!(matches!(
        commands.as_slice(),
        [PluginsCmd::Diff(tui_diff::Cmd::Step { index: 1, .. })]
    ));

    let (mut wheeled, mut commands) = update(Msg::Mouse(scroll(4, 12, false)), &app);
    coalesce_commit_loads(&mut wheeled, &mut commands);
    assert_eq!(diff_focus_of(&wheeled), Some(DIFF_BAND_FOCUS));
    assert!(matches!(
        commands.as_slice(),
        [PluginsCmd::Diff(tui_diff::Cmd::Step { index: 3, .. })]
    ));

    let mut narrow = app.clone();
    narrow.size = Size {
        width: 60,
        height: 20,
    };
    let (mut narrow_clicked, mut commands) = update(Msg::Mouse(click(4, 12)), &narrow);
    coalesce_commit_loads(&mut narrow_clicked, &mut commands);
    assert!(matches!(
        commands.as_slice(),
        [PluginsCmd::Diff(tui_diff::Cmd::Step { index: 1, .. })]
    ));

    let (files, commands) = update(Msg::Mouse(click(4, 3)), &app);
    assert_eq!(diff_focus_of(&files), Some(TREE_FOCUS));
    assert!(commands.is_empty());
}

#[cfg(feature = "diff")]
#[test]
fn batched_commit_navigation_loads_only_the_final_step_and_failure_keeps_the_displayed_step() {
    let mut app = commits_app(6);
    let displayed = diff_ref(&app).diffs().to_vec();
    let displayed_labels = diff_ref(&app).revisions();
    let mut commands = Vec::new();
    for key in [Key::Down, Key::Char('j'), Key::Down] {
        let (next, produced) = update(Msg::Key(key), &app);
        app = next;
        commands.extend(produced);
    }
    assert_eq!(
        commits_ref(&app).cursor(),
        0,
        "the shown step is still loaded"
    );
    assert_eq!(commits_ref(&app).target(), Some(3));
    assert!(commands.is_empty(), "a step load is deferred to coalescing");
    coalesce_commit_loads(&mut app, &mut commands);
    assert!(matches!(
        commands.as_slice(),
        [PluginsCmd::Diff(tui_diff::Cmd::Step { index: 3, .. })]
    ));
    assert_eq!(diff_ref(&app).diffs(), displayed.as_slice());
    assert_eq!(diff_ref(&app).revisions(), displayed_labels);

    let (loaded, _) = update(
        diff_msg(DiffMsg::StepLoaded {
            index: 3,
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("new\n"))].into()),
        }),
        &app,
    );
    assert_eq!(commits_ref(&loaded).cursor(), 3);
    assert_eq!(commits_ref(&loaded).target(), None);
    assert_ne!(diff_ref(&loaded).revisions(), displayed_labels);
    assert_eq!(loaded.workspace.selected, app.workspace.selected);

    let (failed, commands) = update(
        diff_msg(DiffMsg::StepLoaded {
            index: 3,
            result: Err(Box::new(EngineError::Selection(
                base::SelectionError::UnknownArea {
                    name: "missing".to_owned(),
                },
            ))),
        }),
        &app,
    );
    assert!(commands.is_empty());
    assert_eq!(commits_ref(&failed).cursor(), 0);
    assert_eq!(commits_ref(&failed).target(), None);
    assert_eq!(diff_ref(&failed).diffs(), displayed.as_slice());
    assert_eq!(diff_ref(&failed).revisions(), displayed_labels);
    assert!(failed.diagnostic.is_some());
}

#[cfg(feature = "diff")]
#[test]
fn commit_navigation_back_to_loaded_step_cancels_batched_projection() {
    let mut app = commits_app(3);
    let mut commands = Vec::new();
    for key in [Key::Down, Key::Up] {
        let (next, produced) = update(Msg::Key(key), &app);
        app = next;
        commands.extend(produced);
    }
    coalesce_commit_loads(&mut app, &mut commands);
    assert!(commands.is_empty());
    assert!(!app.is_busy());
    assert_eq!(commits_ref(&app).target(), None);
}

#[cfg(feature = "diff")]
#[test]
fn a_scope_change_reloads_the_pending_commit_step_with_the_new_scope() {
    // Load the next step so the picker cursor sits past step 0.
    let (advanced, produced) = update(Msg::Key(Key::Char('j')), &commits_app(3));
    assert!(produced.is_empty());
    let mut commands = Vec::new();
    let mut app = advanced;
    coalesce_commit_loads(&mut app, &mut commands);
    assert!(matches!(
        commands.as_slice(),
        [PluginsCmd::Diff(tui_diff::Cmd::Step { index: 1, .. })]
    ));
    let (app, _) = update(
        diff_msg(DiffMsg::StepLoaded {
            index: 1,
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("B\n"))].into()),
        }),
        &app,
    );
    assert_eq!(commits_ref(&app).cursor(), 1);

    // Changing the scope emits the diff/history request under the new scope.
    let new_selection = Selection::new(vec![SelectionGroup::Path(RepoPath::new("a.rs").unwrap())]);
    let mut requery_app = app.clone();
    let commands = run_request(
        &mut requery_app,
        Request::Requery(QueryChange::Scope(new_selection.clone())),
    );
    let request = match commands.as_slice() {
        [PluginsCmd::Diff(tui_diff::Cmd::History(request))] => request.clone(),
        other => panic!("expected one diff request, got {other:?}"),
    };
    assert_eq!(request.selection, new_selection);

    // The pending step folded from that history load must carry the incoming
    // scope, not the stale committed one.
    let (_, commands) = update(
        diff_msg(DiffMsg::HistoryLoaded {
            request,
            result: Ok((
                commit_steps(3).into(),
                vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into(),
            )),
        }),
        &app,
    );
    let step = commands
        .iter()
        .find_map(|cmd| match cmd {
            PluginsCmd::Diff(tui_diff::Cmd::Step { request, .. }) => Some(request),
            _ => None,
        })
        .expect("a pending commit step load");
    assert_eq!(step.selection, new_selection);
}

#[cfg(feature = "diff")]
#[test]
fn palette_switches_between_range_and_commit_views() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    assert!(has_palette_entry(&app, DIFF, "view.commits"));
    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_command(&mut next, DIFF, "view.commits", &mut commands);
    assert_eq!(
        commands,
        vec![PluginsCmd::Diff(tui_diff::Cmd::History(DiffRequest {
            view: DiffView::Commits,
            ..diff_request()
        }))]
    );
    assert_eq!(diff_ref(&next).revisions(), diff_ref(&app).revisions());
}

#[cfg(feature = "diff")]
#[test]
fn a_snapshot_range_cannot_switch_to_commits() {
    let request = DiffRequest {
        base: ":index".to_owned(),
        target: ":worktree".to_owned(),
        ..diff_request()
    };
    let app = base_app(LoadRequest::Diff {
        base: request.base.clone(),
        target: request.target.clone(),
        mode: request.mode,
        selection: request.selection.clone(),
        view: DiffView::Range,
        merge_base: request.merge_base,
    });
    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request,
            result: Ok(vec![file_diff("a.rs", None, Some("a\n"))].into()),
        }),
        &app,
    );
    let app = settle(next);

    assert!(!has_palette_entry(&app, DIFF, "view.commits"));

    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_command(&mut next, DIFF, "view.commits", &mut commands);
    assert!(commands.is_empty(), "no first-parent load may be issued");
    assert!(next.diagnostic.is_some(), "the refusal is explained");
}

#[cfg(feature = "diff")]
#[test]
fn a_merge_base_range_preserves_the_mode_and_cannot_switch_to_commits() {
    let request = DiffRequest {
        base: "main".to_owned(),
        target: "feature".to_owned(),
        merge_base: true,
        ..diff_request()
    };
    let app = base_app(LoadRequest::Diff {
        base: request.base.clone(),
        target: request.target.clone(),
        mode: request.mode,
        selection: request.selection.clone(),
        view: DiffView::Range,
        merge_base: request.merge_base,
    });
    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request,
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into()),
        }),
        &app,
    );
    let app = settle(next);

    assert!(diff_ref(&app).merge_base);
    assert!(
        diff_ref(&app)
            .request(app.workspace.query.scope.selection.clone())
            .merge_base
    );

    assert!(!has_palette_entry(&app, DIFF, "view.commits"));
    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_command(&mut next, DIFF, "view.commits", &mut commands);
    assert!(commands.is_empty(), "no first-parent load may be issued");
    assert!(next.diagnostic.is_some(), "the refusal is explained");
}

#[cfg(feature = "diff")]
#[test]
fn a_refused_commit_switch_leaves_paging_ready() {
    let app = snapshot_range_app();
    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_command(&mut next, DIFF, "view.commits", &mut commands);
    assert!(commands.is_empty(), "no first-parent load may be issued");
    assert!(next.diagnostic.is_some(), "the refusal is explained");
    assert!(
        !next.plugins.diff.loading,
        "the refusal returns before paging is set to Loading"
    );
}

#[cfg(feature = "diff")]
#[test]
fn a_merge_base_switch_refusal_leaves_paging_ready() {
    let app = merge_base_range_app();
    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_command(&mut next, DIFF, "view.commits", &mut commands);
    assert!(commands.is_empty(), "no first-parent load may be issued");
    assert!(next.diagnostic.is_some(), "the refusal is explained");
    assert!(
        !next.plugins.diff.loading,
        "the refusal returns before paging is set to Loading"
    );
}

#[test]
fn navigation_defers_highlighting_until_settle() {
    let app = show_app(vec![
        source_outline("a.rs", "pub fn a();\n"),
        source_outline("b.rs", "pub fn b();\n"),
    ]);
    let mut fresh = app.clone();
    let show = &mut fresh.plugins.source;
    show.highlight = ShowHighlights::default();
    assert!(
        show_ref(&fresh)
            .active_lines(fresh.workspace.selected.as_ref())
            .is_none()
    );

    let (moved, _) = update(Msg::Key(Key::Down), &fresh);
    assert!(
        show_ref(&moved)
            .active_lines(moved.workspace.selected.as_ref())
            .is_none(),
        "update must not highlight"
    );

    let settled = settle(moved);
    assert!(
        show_ref(&settled)
            .active_lines(settled.workspace.selected.as_ref())
            .is_some()
    );
}

#[test]
fn settle_skips_highlighting_when_the_body_is_hidden() {
    let mut app = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    app.size = Size {
        width: 60,
        height: 30,
    };
    app.plugins.tree.cursor = 0;
    app.focus = Some(TREE_FOCUS);
    let show = &mut app.plugins.source;
    show.highlight = ShowHighlights::default();
    let settled = settle(app);
    assert!(
        show_ref(&settled)
            .active_lines(settled.workspace.selected.as_ref())
            .is_none()
    );
}

#[test]
fn the_highlight_cache_is_bounded() {
    let mut cache: tui_core::util::cache::BoundedCache<usize> =
        tui_core::util::cache::BoundedCache::default();
    for index in 0..(tui_core::util::cache::CAP + 10) {
        let path = RepoPath::new(format!("f{index}.rs")).expect("valid path");
        cache.insert(path, index);
    }
    assert!(!cache.contains_key(&RepoPath::new("f0.rs").expect("valid path")));
    let newest =
        RepoPath::new(format!("f{}.rs", tui_core::util::cache::CAP + 9)).expect("valid path");
    assert!(cache.contains_key(&newest));
}

#[test]
fn mode_picker_defers_the_mode_change_until_selection() {
    let app = two_files();
    let (opened, cmds) = update(Msg::Key(Key::Char('m')), &app);
    assert!(cmds.is_empty());
    assert_eq!(opened.kernel.overlay, Some(MODES_PANE));
    assert_eq!(opened.plugins.modes.cursor, 0);

    let (down, _) = update(Msg::Key(Key::Down), &opened);
    assert_eq!(down.plugins.modes.cursor, 1);
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);

    assert_eq!(app.workspace.query.mode, ProjectionMode::Types);
    assert_eq!(
        chosen.workspace.query.mode,
        ProjectionMode::Types,
        "mode changes on load"
    );
    assert_eq!(chosen.kernel.overlay, None);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            mode: ProjectionMode::Signatures,
            ..show_request()
        }))]
    );
}

#[cfg(feature = "diff")]
#[test]
fn mode_picker_on_a_diff_keeps_the_diff_shape() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let (opened, _) = update(Msg::Key(Key::Char('m')), &app);
    let (down, _) = update(Msg::Key(Key::Down), &opened);
    let (_, cmds) = update(Msg::Key(Key::Enter), &down);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Diff(tui_diff::Cmd::Range(DiffRequest {
            mode: ProjectionMode::Signatures,
            ..diff_request()
        }))]
    );
}

#[test]
fn the_mode_picker_lists_the_modes() {
    let app = two_files();
    let opened = press(&app, &[Key::Char('m')]);
    assert_eq!(opened.kernel.overlay, Some(MODES_PANE));
    let text = buffer_text(&render(&opened, 100, 20));
    assert!(text.contains("types"), "{text}");
    assert!(text.contains("signatures"), "{text}");
    assert!(text.contains("tests"), "{text}");
}

#[test]
fn the_mode_picker_reaches_tests_and_clamps_at_the_last_mode() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('m')), &app);
    let (down, _) = update(Msg::Key(Key::Down), &opened);
    let (down_again, _) = update(Msg::Key(Key::Down), &down);
    let (_, cmds) = update(Msg::Key(Key::Enter), &down_again);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            mode: ProjectionMode::Tests,
            ..show_request()
        }))]
    );

    let (clamped, _) = update(Msg::Key(Key::Down), &down_again);
    let (_, cmds) = update(Msg::Key(Key::Enter), &clamped);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            mode: ProjectionMode::Tests,
            ..show_request()
        }))]
    );
}

#[test]
fn tree_resize_keys_step_clamp_and_reset() {
    let app = two_files();
    let (grow, _) = update(Msg::Key(Key::Char(']')), &app);
    assert_eq!(grow.chrome.tree_percent, TREE_DEFAULT_PERCENT + TREE_STEP);
    let (shrink, _) = update(Msg::Key(Key::Char('[')), &grow);
    assert_eq!(shrink.chrome.tree_percent, TREE_DEFAULT_PERCENT);

    let mut widest = app.clone();
    widest.chrome.tree_percent = TREE_MAX_PERCENT;
    let (widest, _) = update(Msg::Key(Key::Char(']')), &widest);
    assert_eq!(widest.chrome.tree_percent, TREE_MAX_PERCENT);

    let mut narrowest = app.clone();
    narrowest.chrome.tree_percent = TREE_MIN_PERCENT;
    let (narrowest, _) = update(Msg::Key(Key::Char('[')), &narrowest);
    assert_eq!(narrowest.chrome.tree_percent, TREE_MIN_PERCENT);

    let (reset, _) = update(Msg::Key(Key::Char('\\')), &grow);
    assert_eq!(reset.chrome.tree_percent, TREE_DEFAULT_PERCENT);
}

#[test]
fn a_wider_tree_still_leaves_a_usable_body() {
    let mut app = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    app.chrome.tree_percent = TREE_MAX_PERCENT;
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Files"), "{text}");
    assert!(text.contains("pub fn a();"), "{text}");
}

#[cfg(feature = "diff")]
#[test]
fn tab_toggles_the_tree_and_both_diff_panes() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let (step, _) = update(Msg::Key(Key::Tab), &app);
    assert_eq!(diff_focus_of(&step), Some(DIFF_BODY_FOCUS));
    let (step, _) = update(Msg::Key(Key::Tab), &step);
    assert_eq!(diff_focus_of(&step), Some(TREE_FOCUS));
    let (step, _) = update(Msg::Key(Key::BackTab), &step);
    assert_eq!(diff_focus_of(&step), Some(DIFF_BODY_FOCUS));

    let app = two_files();
    let (step, _) = update(Msg::Key(Key::Tab), &app);
    assert_eq!(step.focus, Some(SOURCE_FOCUS));
    let (step, _) = update(Msg::Key(Key::BackTab), &step);
    assert_eq!(step.focus, Some(TREE_FOCUS));
}

#[test]
fn help_toggles_and_swallows_navigation() {
    let app = two_files();
    let (help, _) = update(Msg::Key(Key::Char('?')), &app);
    assert_eq!(help.kernel.overlay, Some(HELP_PANE));

    let (still, _) = update(Msg::Key(Key::Char('j')), &help);
    assert_eq!(still.plugins.tree.cursor, help.plugins.tree.cursor);
    assert_eq!(still.kernel.overlay, Some(HELP_PANE));

    let (closed, _) = update(Msg::Key(Key::Esc), &help);
    assert_eq!(closed.kernel.overlay, None);
}

#[test]
fn revision_prompt_prefills_the_current_revision() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    let prompt = opened.plugins.prompt.open.as_ref().expect("prompt");
    assert_eq!(prompt.input.text, "HEAD");
    assert_eq!(prompt.input.cursor, 4);
    assert_eq!(prompt.label, "revision");
}

#[test]
fn confirming_a_revision_emits_a_load_with_that_revision() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    let mut state = opened;
    for _ in 0.."HEAD".len() {
        let (next, _) = update(Msg::Key(Key::Backspace), &state);
        state = next;
    }
    for character in "HEAD~2".chars() {
        let (next, _) = update(Msg::Key(Key::Char(character)), &state);
        state = next;
    }
    let (applied, cmds) = update(Msg::Key(Key::Enter), &state);
    assert!(applied.plugins.prompt.open.is_none());
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            revision: "HEAD~2".to_owned(),
            ..show_request()
        }))]
    );
}

#[test]
fn escaping_a_revision_prompt_emits_nothing() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    let (closed, cmds) = update(Msg::Key(Key::Esc), &opened);
    assert!(closed.plugins.prompt.open.is_none());
    assert!(cmds.is_empty());
}

#[cfg(feature = "diff")]
#[test]
fn a_cancelled_prompt_does_not_wedge_input() {
    // A show revision prompt: Esc cancels it.
    let mut app = folder_app();
    focus_content(&mut app);
    let before = show_ref(&app).body.cursor_line;

    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    assert!(opened.plugins.prompt.open.is_some(), "r opens the prompt");
    assert_eq!(
        opened.kernel.overlay,
        Some(PROMPT_PANE),
        "the prompt occupies the overlay slot"
    );

    let (cancelled, _) = update(Msg::Key(Key::Esc), &opened);
    assert!(cancelled.plugins.prompt.open.is_none(), "Esc cancels it");
    assert_eq!(
        cancelled.kernel.overlay, None,
        "the overlay slot clears with the prompt"
    );

    // j is no longer swallowed: it moves the show cursor again.
    let (moved, _) = update(Msg::Key(Key::Char('j')), &cancelled);
    assert!(
        show_ref(&moved).body.cursor_line > before,
        "j moves the cursor after the prompt is cancelled"
    );

    // q is no longer swallowed: it quits.
    let (quit, _) = update(Msg::Key(Key::Char('q')), &cancelled);
    assert!(quit.chrome.quit, "q quits after the prompt is cancelled");

    // Enter with an empty value cancels the prompt just like Esc.
    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    let mut emptied = opened;
    for _ in 0.."HEAD".len() {
        let (next, _) = update(Msg::Key(Key::Backspace), &emptied);
        emptied = next;
    }
    let (empty_cancelled, cmds) = update(Msg::Key(Key::Enter), &emptied);
    assert!(empty_cancelled.plugins.prompt.open.is_none());
    assert!(cmds.is_empty(), "an empty value emits no load");
    assert_eq!(
        empty_cancelled.kernel.overlay, None,
        "the slot clears on an empty confirm too"
    );
    let (quit, _) = update(Msg::Key(Key::Char('q')), &empty_cancelled);
    assert!(quit.chrome.quit, "q quits after an empty-value cancel");

    // The diff base and target prompts cancel the same way.
    for key in [Key::Char('b'), Key::Char('t')] {
        let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let (opened, _) = update(Msg::Key(key), &app);
        assert!(
            opened.plugins.prompt.open.is_some(),
            "{key:?} opens a prompt"
        );
        let (cancelled, _) = update(Msg::Key(Key::Esc), &opened);
        assert!(cancelled.plugins.prompt.open.is_none());
        assert_eq!(
            cancelled.kernel.overlay, None,
            "the diff prompt clears on Esc"
        );
        let (quit, _) = update(Msg::Key(Key::Char('q')), &cancelled);
        assert!(quit.chrome.quit, "q quits after a cancelled diff prompt");
    }
}

#[cfg(feature = "diff")]
#[test]
fn diff_revision_keys_target_base_and_target_fields() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("b\n"))]);
    let (base, _) = update(Msg::Key(Key::Char('b')), &app);
    assert_eq!(
        base.plugins.prompt.open.as_ref().map(|p| p.label),
        Some("base")
    );
    let (target, _) = update(Msg::Key(Key::Char('t')), &app);
    assert_eq!(
        target.plugins.prompt.open.as_ref().map(|p| p.label),
        Some("target")
    );
}

#[test]
fn the_revision_prompt_renders_its_label_and_text() {
    let mut app = two_files();
    app.plugins.prompt.open = Some(crate::prompt::Open {
        from: crate::plugin::SOURCE,
        tag: 0,
        label: "revision",
        input: TextInput::new("HEAD~2"),
    });
    app.kernel.overlay = Some(PROMPT_PANE);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("revision"), "{text}");
    assert!(text.contains("HEAD~2"), "{text}");
}

#[test]
fn scope_key_opens_the_chooser_and_requests_areas() {
    let app = two_files();
    let (opened, cmds) = update(Msg::Key(Key::Char('s')), &app);
    assert_eq!(opened.kernel.overlay, Some(SCOPE_PANE));
    assert!(
        opened.plugins.scope.areas.is_none(),
        "the chooser starts loading"
    );
    assert_eq!(opened.plugins.scope.cursor, 0);
    assert_eq!(opened.plugins.scope.error, None);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Scope(crate::scope_chooser::Cmd::LoadAreas)]
    );
}

#[test]
fn choosing_all_from_the_chooser_loads_everything() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let (loaded, _) = update(scope_msg(ScopeMsg::AreasLoaded(Ok(area_set()))), &opened);
    assert!(loaded.plugins.scope.areas.is_some());
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &loaded);
    assert_eq!(chosen.kernel.overlay, None);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            selection: Selection::all(),
            ..show_request()
        }))]
    );
}

#[test]
fn selecting_an_area_loads_that_area() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let (loaded, _) = update(scope_msg(ScopeMsg::AreasLoaded(Ok(area_set()))), &opened);
    let (down, _) = update(Msg::Key(Key::Down), &loaded);
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);
    assert_eq!(chosen.kernel.overlay, None);
    let expected = Selection::new(vec![SelectionGroup::Area(
        Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
    )]);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            selection: expected,
            ..show_request()
        }))]
    );
}

#[test]
fn a_config_error_keeps_the_chooser_open_with_a_message() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let error = EngineError::Selection(base::SelectionError::UnknownArea {
        name: "x".to_owned(),
    });
    let (loaded, _) = update(
        scope_msg(ScopeMsg::AreasLoaded(Err(Box::new(error)))),
        &opened,
    );
    assert_eq!(loaded.kernel.overlay, Some(SCOPE_PANE));
    assert!(loaded.plugins.scope.error.is_some());
}

#[test]
fn literal_path_entry_builds_a_path_selection() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let (loaded, _) = update(scope_msg(ScopeMsg::AreasLoaded(Ok(area_set()))), &opened);
    let mut state = loaded;
    for _ in 0..3 {
        let (next, _) = update(Msg::Key(Key::Down), &state);
        state = next;
    }
    let (input, _) = update(Msg::Key(Key::Enter), &state);
    assert!(input.plugins.scope.input.is_some());
    let mut typed = input;
    for character in "src/lib.rs".chars() {
        let (next, _) = update(Msg::Key(Key::Char(character)), &typed);
        typed = next;
    }
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &typed);
    assert_eq!(chosen.kernel.overlay, None);
    let expected = Selection::new(vec![SelectionGroup::Path(
        RepoPath::new("src/lib.rs").unwrap(),
    )]);
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            selection: expected,
            ..show_request()
        }))]
    );
}

#[cfg(feature = "diff")]
#[test]
fn search_finds_matches_and_steps_them() {
    let app = show_app(vec![source_outline("a.rs", "alpha\nbeta\nalpha again\n")]);
    let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
    assert_eq!(opened.kernel.overlay, Some(SEARCH_PANE));

    let mut state = opened;
    for character in "alpha".chars() {
        let (next, _) = update(Msg::Key(Key::Char(character)), &state);
        state = next;
    }
    assert_eq!(
        state
            .workspace
            .search
            .as_ref()
            .expect("live search")
            .matches
            .len(),
        2
    );

    let (committed, _) = update(Msg::Key(Key::Enter), &state);
    assert_eq!(committed.kernel.overlay, None);
    let (stepped, _) = update(Msg::Key(Key::Char('n')), &committed);
    assert_eq!(stepped.workspace.search.as_ref().unwrap().cursor, 1);
    let (back, _) = update(Msg::Key(Key::Char('N')), &stepped);
    assert_eq!(back.workspace.search.as_ref().unwrap().cursor, 0);
}

#[cfg(feature = "diff")]
#[test]
fn switching_files_recomputes_a_committed_search() {
    let mut app = show_app(vec![
        source_outline("a.rs", "alpha\n"),
        source_outline("b.rs", "beta\nalpha\n"),
    ]);
    app.workspace.search = Some(Search {
        needle: "alpha".to_owned(),
        matches: vec![SearchMatch {
            pane: SearchPane::Source,
            line: 1,
            start: 0,
            end: 5,
        }],
        cursor: 0,
    });
    app.workspace.selected = Some(RepoPath::new("b.rs").unwrap());
    let app = settle(app);
    let search = app.kernel.workspace.search.clone().expect("search");
    assert_eq!(search.matches.len(), 1);
    assert_eq!(search.matches[0].line, 2, "the match moved to the new file");
}

#[test]
fn q_types_into_a_search_input_instead_of_quitting() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
    let (typed, _) = update(Msg::Key(Key::Char('q')), &opened);
    assert!(!typed.chrome.quit);
    assert_eq!(typed.plugins.search.input.text, "q");
    assert_eq!(typed.kernel.overlay, Some(SEARCH_PANE));
}

#[test]
fn the_palette_and_finder_render_their_lists() {
    let app = two_files();
    let palette = press(&app, &[Key::CtrlP]);
    let text = buffer_text(&render(&palette, 100, 20));
    assert!(text.contains("commands"), "{text}");
    assert!(text.contains("Switch types"), "{text}");

    let finder = press(&app, &[Key::CtrlF]);
    let text = buffer_text(&render(&finder, 100, 20));
    assert!(text.contains("find file"), "{text}");
    assert!(text.contains("a.rs"), "{text}");
}

const TREE_X: u16 = 2;
const TREE_Y: u16 = 2;
const CONTENT_X: u16 = 50;

#[test]
fn page_keys_move_the_cursor_in_the_tree_and_the_body_cursor() {
    let app = show_app(vec![
        source_outline("a.rs", "one\ntwo\nthree\nfour\nfive\n"),
        source_outline("b.rs", "x\n"),
    ]);
    let (down, _) = update(Msg::Key(Key::PageDown), &app);
    assert!(down.plugins.tree.cursor > app.plugins.tree.cursor);

    let mut body = app.clone();
    focus_content(&mut body);
    let (moved, _) = update(Msg::Key(Key::PageDown), &body);
    assert_eq!(
        show_ref(&moved).body.cursor_line,
        5,
        "PageDown moves the body cursor to the last visible row"
    );
}

#[test]
fn the_body_cursor_is_clamped_to_the_content() {
    let mut app = show_app(vec![source_outline("a.rs", "one\ntwo\nthree\n")]);
    focus_content(&mut app);
    for _ in 0..10 {
        let (next, _) = update(Msg::Key(Key::Char('j')), &app);
        app = next;
    }
    assert_eq!(show_ref(&app).body.cursor_line, 3);
    for _ in 0..10 {
        let (next, _) = update(Msg::Key(Key::Char('k')), &app);
        app = next;
    }
    assert_eq!(show_ref(&app).body.cursor_line, 1);
}

#[test]
fn clicking_a_file_selects_it() {
    let app = two_files();
    let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
    assert_eq!(selected(&next).as_deref(), Some("b.rs"));
    assert_eq!(next.plugins.tree.cursor, 1);
    assert_eq!(next.focus, Some(TREE_FOCUS));
}

#[test]
fn clicking_a_directory_folds_it() {
    let app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("src/lib.rs", "l\n"),
    ]);
    let (folded, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
    let labels: Vec<&str> = folded
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["a.rs", "src"]);
}

#[test]
fn the_wheel_scrolls_the_tree_without_moving_the_selection() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let app = show_app(files);
    assert_eq!(app.plugins.tree.scroll, 0);
    assert_eq!(selected(&app).as_deref(), Some("f00.rs"));

    let (scrolled, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &app);
    assert_eq!(
        scrolled.plugins.tree.scroll, 3,
        "the wheel moves the viewport"
    );
    assert_eq!(
        scrolled.plugins.tree.cursor, 0,
        "the wheel does not move the cursor"
    );
    assert_eq!(
        selected(&scrolled).as_deref(),
        Some("f00.rs"),
        "the wheel does not change the selection"
    );

    let (up, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, true)), &scrolled);
    assert_eq!(up.plugins.tree.scroll, 0, "scrolling up clamps at the top");
}

#[test]
fn moving_the_tree_cursor_after_a_scroll_reveals_it() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let app = show_app(files);
    let (scrolled, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &app);
    assert_eq!(scrolled.plugins.tree.scroll, 3);

    let moved = press(&scrolled, &[Key::Char('j'); 30]);
    assert_eq!(moved.plugins.tree.cursor, 30);
    assert_eq!(
        moved.plugins.tree.scroll, 5,
        "the viewport follows the cursor"
    );
}

#[test]
fn clicking_the_fold_gutter_toggles_without_moving_the_cursor() {
    let app = show_app_mode(
        vec![two_function_outline("a.rs")],
        ProjectionMode::Signatures,
    );
    let buffer = render(&app, 100, 20);
    let second = row_with(&buffer, "fn b").expect("the second fold row");
    let marker_x = cell_x_of(&buffer, second, "+").expect("a fold marker");

    let (opened, _) = update(Msg::Mouse(click(marker_x, second)), &app);
    assert_eq!(
        show_ref(&opened).body.cursor_line,
        1,
        "the marker click leaves the cursor where it was"
    );
    assert!(
        matches!(
            show_ref(&opened).visible_rows()[1],
            VisibleRow::Source { line: 4 }
        ),
        "the clicked declaration is now open"
    );
    let first = show_ref(&opened).fold_marker(
        opened.workspace.selected.as_ref(),
        &show_ref(&opened).visible_rows()[0],
    );
    assert_eq!(first, Some('+'), "the other declaration stays folded");
}

#[test]
fn clicking_a_source_line_moves_the_cursor() {
    let app = show_app_mode(
        vec![two_function_outline("a.rs")],
        ProjectionMode::Signatures,
    );
    let buffer = render(&app, 100, 20);
    let second = row_with(&buffer, "fn b").expect("the second fold row");
    let text_x = cell_x_of(&buffer, second, "fn b").expect("the folded signature text");

    let (placed, _) = update(Msg::Mouse(click(text_x, second)), &app);
    assert_eq!(
        show_ref(&placed).body.cursor_line,
        4,
        "clicking a line moves the cursor to it"
    );
}

#[test]
fn clicking_the_pane_padding_does_not_toggle_a_fold() {
    let app = show_app_mode(
        vec![two_function_outline("a.rs")],
        ProjectionMode::Signatures,
    );
    let (clicked, _) = update(Msg::Mouse(click(30, 2)), &app);
    assert!(
        matches!(
            show_ref(&clicked).visible_rows()[0],
            VisibleRow::Fold { .. }
        ),
        "a click left of the pane's inner area is not a marker click"
    );
}

#[test]
fn shrinking_the_tree_keeps_the_cursor_visible() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let app = press(&show_app(files), &[Key::Char('G')]);
    assert_eq!(app.plugins.tree.cursor, 39);

    let (resized, _) = update(
        Msg::Resize {
            width: 100,
            height: 6,
        },
        &app,
    );
    let resized = settle(resized);
    let height = resized.plugins.tree.viewport;
    assert!(height >= 1, "the tree still has a viewport");
    assert!(
        resized.plugins.tree.scroll <= resized.plugins.tree.cursor
            && resized.plugins.tree.cursor < resized.plugins.tree.scroll + height,
        "the cursor stays inside the viewport after a shrink"
    );
}

#[test]
fn a_selection_scrolls_the_hidden_tree_to_the_file() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let mut app = show_app(files);
    app.size = Size {
        width: 60,
        height: 20,
    };
    focus_content(&mut app);

    select_file(&mut app, RepoPath::new("f39.rs").expect("valid path"));

    let height = tree_viewport_height(&app);
    assert!(height >= 1, "a hidden tree still has a drawing height");
    assert_eq!(app.plugins.tree.cursor, 39);
    assert!(
        app.plugins.tree.scroll <= app.plugins.tree.cursor
            && app.plugins.tree.cursor < app.plugins.tree.scroll + height,
        "the hidden tree is scrolled to the revealed file"
    );
}

#[test]
fn growing_the_tree_keeps_a_scrolled_away_viewport() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let app = show_app(files);
    let (scrolled, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &app);
    assert_eq!(scrolled.plugins.tree.scroll, 3);

    let (resized, _) = update(
        Msg::Resize {
            width: 100,
            height: 40,
        },
        &scrolled,
    );
    let resized = settle(resized);
    assert_eq!(
        resized.plugins.tree.scroll, 3,
        "growing the tree does not re-reveal a scrolled-away cursor"
    );
}

#[test]
fn a_tree_click_uses_the_scrolled_offset() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let app = show_app(files);
    let (scrolled, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &app);
    assert_eq!(scrolled.plugins.tree.scroll, 3);

    let (clicked, _) = update(Msg::Mouse(click(TREE_X, TREE_Y)), &scrolled);
    assert_eq!(
        clicked.plugins.tree.cursor, 3,
        "the click maps to the scrolled row"
    );
    assert_eq!(selected(&clicked).as_deref(), Some("f03.rs"));
}

#[test]
fn the_wheel_scrolls_the_show_pane_from_its_padding() {
    let app = show_app(vec![source_outline(
        "a.rs",
        "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
    )]);
    let (down, _) = update(Msg::Mouse(scroll(30, 5, false)), &app);
    assert_eq!(
        show_ref(&down).body.scroll,
        3,
        "the wheel acts anywhere in the pane, including its padding"
    );
}

#[test]
fn the_wheel_scrolls_the_content_and_focuses_it() {
    let app = show_app(vec![source_outline(
        "a.rs",
        "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n",
    )]);
    let (down, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &app);
    assert_eq!(down.focus, Some(SOURCE_FOCUS));
    assert_eq!(show_ref(&down).body.scroll, 3);
    assert_eq!(
        show_ref(&down).body.cursor_line,
        1,
        "the wheel scrolls the viewport without moving the cursor"
    );
    let (up, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, true)), &down);
    assert_eq!(show_ref(&up).body.scroll, 0);
    assert_eq!(show_ref(&up).body.cursor_line, 1);
}

#[cfg(feature = "diff")]
#[test]
fn the_wheel_scrolls_a_diff_and_clicking_it_focuses() {
    let app = diff_app(vec![file_diff(
        "a.rs",
        Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"),
        Some("1\n2\n3\n4\n5\n6\n7\n8\n9\nX\n"),
    )]);
    let (scrolled, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &app);
    assert_eq!(diff_focus_of(&scrolled), Some(DIFF_BODY_FOCUS));
    assert!(diff_ref(&scrolled).body_scroll() > 0);

    let (clicked, _) = update(Msg::Mouse(click(CONTENT_X, 5)), &app);
    assert_eq!(diff_focus_of(&clicked), Some(DIFF_BODY_FOCUS));
    assert_eq!(diff_ref(&clicked).body_scroll(), 0);
}

#[test]
fn mouse_is_ignored_while_an_overlay_is_open() {
    let mut app = two_files();
    app.kernel.overlay = Some(HELP_PANE);
    let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
    assert_eq!(next.plugins.tree.cursor, app.plugins.tree.cursor);
    assert_eq!(next.workspace.selected, app.workspace.selected);
}

#[test]
fn a_click_on_the_header_or_a_divider_does_nothing() {
    let app = two_files();
    let (header, _) = update(Msg::Mouse(click(5, 0)), &app);
    assert_eq!(header.plugins.tree.cursor, app.plugins.tree.cursor);

    let mut body_focused = app.clone();
    focus_content(&mut body_focused);
    let (divider, _) = update(Msg::Mouse(click(29, 5)), &body_focused);
    assert_eq!(divider.focus, Some(SOURCE_FOCUS));
    assert_eq!(divider.workspace.selected, app.workspace.selected);
}

#[test]
fn the_status_bar_shows_mode_scope_and_hints() {
    let app = show_app(vec![source_outline("a.rs", "x\n")]);
    let buffer = render(&app, 120, 20);
    let header = buffer_row(&buffer, 0);
    let footer = buffer_row(&buffer, 19);
    assert!(header.contains("types"), "{header}");
    assert!(header.contains("HEAD"), "{header}");
    assert!(header.contains("scope:"), "{header}");
    assert!(!header.contains("a.rs"), "{header}");
    assert!(footer.contains("a.rs"), "{footer}");
    assert!(footer.contains("1 of 1 lines"), "{footer}");
    assert!(footer.contains("help"), "{footer}");
}

#[test]
fn the_status_bar_counts_visible_rows_against_source_lines() {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    let text = buffer_text(&render(&app, 120, 20));
    assert!(text.contains("3 of 6 lines"), "{text}");
}

#[test]
fn narrow_chrome_keeps_context_and_the_selected_path() {
    let app = show_app(vec![source_outline("src/very/deep/file.rs", "x\n")]);
    let buffer = render(&app, 60, 20);
    let header = buffer_row(&buffer, 0);
    let footer = buffer_row(&buffer, 19);
    assert!(header.contains("types"), "{header}");
    assert!(header.contains("HEAD"), "{header}");
    assert!(footer.contains("file.rs"), "{footer}");
    assert!(footer.contains("lines"), "{footer}");
    assert!(footer.contains("help"), "{footer}");
    assert_eq!(
        buffer_text(&buffer)
            .matches("src/very/deep/file.rs")
            .count(),
        1
    );
}

#[cfg(feature = "diff")]
#[test]
fn the_status_bar_counts_diff_changes() {
    let app = diff_app(vec![file_diff(
        "a.rs",
        Some("one\ntwo\n"),
        Some("one\n2\n"),
    )]);
    let text = buffer_text(&render(&app, 120, 20));
    assert!(text.contains("+1"), "{text}");
    assert!(text.contains("−1"), "{text}");
}

#[test]
fn a_diagnostic_renders_as_a_toast() {
    let mut app = two_files();
    app.diagnostic = Some(Diagnostic::new("boom".to_owned()));
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("boom"), "{text}");
}

#[test]
fn a_load_marks_the_model_busy_until_it_completes() {
    let app = two_files();
    assert!(!app.is_busy(), "an installed model is idle");

    let (opened, cmds) = update(Msg::Key(Key::Char('m')), &app);
    assert!(cmds.is_empty(), "opening the picker emits no effect");
    let (down, _) = update(Msg::Key(Key::Down), &opened);
    let (loading, cmds) = update(Msg::Key(Key::Enter), &down);
    assert!(!cmds.is_empty());
    assert!(loading.is_busy(), "an emitted load is busy");

    let (done, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![source_outline("a.rs", "a\n"), source_outline("b.rs", "b\n")].into()),
        }),
        &loading,
    );
    assert!(!done.is_busy(), "completion clears the busy state");
}

#[test]
fn a_tick_advances_the_spinner_and_expires_a_diagnostic() {
    let mut app = two_files();
    app.diagnostic = Some(Diagnostic::new("boom".to_owned()));
    let before = app.chrome.spinner;
    let (next, _) = update(Msg::Tick, &app);
    assert_eq!(next.chrome.spinner, before.wrapping_add(1));

    for _ in 0..DIAGNOSTIC_TICKS {
        let (ticked, _) = update(Msg::Tick, &app);
        app = ticked;
    }
    assert!(app.diagnostic.is_none());
}

#[test]
fn a_wide_terminal_shows_the_tree_and_the_source() {
    let app = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Files"), "tree pane missing: {text}");
    assert!(
        text.contains(" Source "),
        "source pane title missing: {text}"
    );
    assert!(text.contains("pub fn a();"), "body missing: {text}");
}

#[test]
fn the_body_shows_the_files_real_source_and_counts_its_lines() {
    let app = show_app(vec![source_outline("a.rs", "alpha\nbeta\n")]);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("alpha"), "missing alpha: {text}");
    assert!(text.contains("beta"), "missing beta: {text}");
    assert_eq!(
        show_ref(&app).active_source(app.workspace.selected.as_ref()),
        Some("alpha\nbeta\n")
    );
    assert_eq!(show_ref(&app).line_count(), 2);
}

#[test]
fn the_body_renders_the_source_not_the_canonical_projection() {
    let file = source_outline_with("a.rs", "real source line\n", "PROJECTED\n");
    let app = show_app(vec![file]);
    assert_eq!(
        show_ref(&app).active_source(app.workspace.selected.as_ref()),
        Some("real source line\n")
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("real source line"), "source missing: {text}");
    assert!(
        !text.contains("PROJECTED"),
        "the canonical projection must not be rendered: {text}"
    );
}

#[test]
fn a_folded_signature_child_hides_its_body_and_counts_as_one_row() {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    let show = show_ref(&app);

    assert_eq!(show.source_line_count(app.workspace.selected.as_ref()), 6);
    assert_eq!(
        show.line_count(),
        3,
        "container line, fold row, close brace"
    );
    assert_eq!(show.line_count(), show.visible_rows().len());
    assert_eq!(
        show.visible_rows(),
        &[
            VisibleRow::Source { line: 1 },
            VisibleRow::Fold {
                line: 2,
                summary_line: 2,
                region_index: 1,
                hidden: 3,
                dropped: false,
            },
            VisibleRow::Source { line: 6 },
        ]
    );
}

#[test]
fn a_collapsed_attributed_signature_summarizes_the_declaration_line() {
    let app = show_app(vec![attributed_child_outline("a.rs", ATTRIBUTED_SOURCE)]);
    let show = show_ref(&app);

    assert_eq!(
        show.visible_rows()[1],
        VisibleRow::Fold {
            line: 2,
            summary_line: 3,
            region_index: 1,
            hidden: 3,
            dropped: false,
        }
    );
    assert_eq!(
        show.visible_rows()[1].source_line(),
        3,
        "the rendered line is the declaration's own line"
    );

    let buffer = render(&app, 100, 20);
    let row = row_with(&buffer, "fn tested()").expect("the declaration line is rendered");
    let text = buffer_row(&buffer, row);
    assert!(
        !text.contains("#[test]"),
        "the attribute must not summarize the fold: {text}"
    );
    assert!(
        text.contains("fn tested() {"),
        "the signature line is the summary: {text}"
    );
    assert!(
        text.contains("3     fn tested()"),
        "the gutter shows the declaration line: {text}"
    );
    assert!(
        text.contains("⋯ 3 lines"),
        "the hidden count is unchanged: {text}"
    );
}

#[test]
fn a_collapsed_test_function_in_signatures_mode_shows_its_signature_line() {
    let app = show_app_mode(
        vec![test_function_outline("a.rs", TEST_SOURCE)],
        ProjectionMode::Signatures,
    );
    let show = show_ref(&app);

    assert_eq!(
        show.visible_rows()[0],
        VisibleRow::Fold {
            line: 1,
            summary_line: 2,
            region_index: 0,
            hidden: 3,
            dropped: false,
        }
    );

    let buffer = render(&app, 100, 20);
    let row = row_with(&buffer, "fn decode_source()").expect("the declaration line is rendered");
    let text = buffer_row(&buffer, row);
    assert!(
        !text.contains("#[test]"),
        "the attribute must not summarize the fold: {text}"
    );
    assert!(
        text.contains("fn decode_source() {"),
        "the test's signature line is the summary: {text}"
    );
    assert!(
        text.contains("2 fn decode_source()"),
        "the gutter shows the declaration line: {text}"
    );
}

#[test]
fn the_fold_suffix_widens_the_horizontal_scroll_range() {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    let show = show_ref(&app);

    let fold_line = NESTED_SOURCE.lines().nth(1).expect("the folded signature");
    let expected = fold_line.chars().count() + "  ⋯ 3 lines".chars().count();
    assert_eq!(
        show.max_line_width(app.workspace.selected.as_ref()),
        expected,
        "max width must include the fold suffix"
    );
}

#[test]
fn a_dropped_declaration_folds_and_marks_the_row_dropped() {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        false,
    )]);
    let show = show_ref(&app);

    let fold = show
        .visible_rows()
        .iter()
        .find_map(|row| match row {
            VisibleRow::Fold {
                line,
                hidden,
                dropped,
                ..
            } => Some((*line, *hidden, *dropped)),
            VisibleRow::Source { .. } => None,
        })
        .expect("a fold row for the dropped child");
    assert_eq!(fold, (2, 3, true), "the child is dropped by the mode");
}

#[test]
fn a_dropped_container_folds_to_one_row_tagged_hidden_in_mode() {
    let app = show_app(vec![dropped_container_outline("a.rs", NESTED_SOURCE)]);
    let show = show_ref(&app);

    assert_eq!(
        show.line_count(),
        1,
        "a dropped container is one fold row, not an open body"
    );
    assert_eq!(
        show.visible_rows(),
        &[VisibleRow::Fold {
            line: 1,
            summary_line: 1,
            region_index: 0,
            hidden: 5,
            dropped: true,
        }]
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(
        text.contains("hidden in types"),
        "the dropped tag is missing: {text}"
    );
    assert!(
        !text.contains("let hidden"),
        "the dropped body must be absent: {text}"
    );
    assert!(
        !text.contains("42"),
        "the dropped body must be absent: {text}"
    );
}

#[test]
fn is_open_honors_default_open_and_a_sticky_override() {
    let path = RepoPath::new("a.rs").expect("valid path");
    let mut app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    let selected = app.kernel.workspace.selected.clone();
    let show = &mut app.plugins.source;
    show.ensure_visible_rows(selected.as_ref());
    let regions = show
        .folds
        .regions(&path)
        .expect("fold regions are cached")
        .to_vec();

    assert!(
        show.fold_state.is_open(&path, &regions[0]),
        "a container is open by default"
    );
    assert!(
        !show.fold_state.is_open(&path, &regions[1]),
        "a method is folded by default"
    );

    show.fold_state.set(&path, &regions[1].stable_key, true);
    assert!(
        show.fold_state.is_open(&path, &regions[1]),
        "the override wins over default_open"
    );
    show.ensure_visible_rows(selected.as_ref());
    assert_eq!(
        show.line_count(),
        6,
        "opening the child reveals every source line"
    );
}

#[test]
fn a_collapsed_container_hides_a_nested_collapse_entirely() {
    let path = RepoPath::new("a.rs").expect("valid path");
    let mut app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    let selected = app.kernel.workspace.selected.clone();
    let show = &mut app.plugins.source;
    show.fold_state.set(&path, "impl Widget", false);
    show.ensure_visible_rows(selected.as_ref());

    assert_eq!(show.visible_rows().len(), 1, "only the outer fold row");
    assert_eq!(
        show.visible_rows(),
        &[VisibleRow::Fold {
            line: 1,
            summary_line: 1,
            region_index: 0,
            hidden: 5,
            dropped: false,
        }]
    );
}

#[test]
fn a_single_line_region_folds_with_zero_hidden_lines() {
    let source = "trait Greeter {\n    fn hello(&self);\n}\n";
    let app = show_app(vec![nested_outline("a.rs", source, (0, 2), (1, 1), true)]);
    let show = show_ref(&app);

    assert_eq!(show.line_count(), 3);
    assert_eq!(
        show.visible_rows()[1],
        VisibleRow::Fold {
            line: 2,
            summary_line: 2,
            region_index: 1,
            hidden: 0,
            dropped: false,
        }
    );
}

#[cfg(feature = "diff")]
#[test]
fn a_match_inside_a_fold_resolves_to_the_fold_row() {
    let source = "impl Widget {\n    fn size(&self) -> usize {\n        let needle = 1;\n        needle\n    }\n}\n";
    let mut app = show_app(vec![nested_outline("a.rs", source, (0, 5), (1, 4), true)]);
    commit_search(&mut app, "needle");
    let show = show_ref(&app);

    assert_eq!(
        app.workspace.search.as_ref().expect("search").matches.len(),
        2,
        "both matches are in the hidden body"
    );
    assert_eq!(show.visible_row_for_source_line(1), Some(0));
    assert_eq!(show.visible_row_for_source_line(2), Some(1), "fold row");
    assert_eq!(show.visible_row_for_source_line(3), Some(1), "hidden body");
    assert_eq!(show.visible_row_for_source_line(4), Some(1), "hidden body");
    assert_eq!(show.visible_row_for_source_line(6), Some(2));
    assert_eq!(show.visible_row_for_source_line(7), None);
}

#[cfg(feature = "diff")]
#[test]
fn stepping_to_a_match_inside_a_fold_opens_the_enclosing_regions() {
    let source = "impl Widget {\n    fn size(&self) -> usize {\n        let needle = 1;\n        needle\n    }\n}\n";
    let mut app = show_app(vec![nested_outline("a.rs", source, (0, 5), (1, 4), true)]);
    focus_content(&mut app);
    let folded = press(&app, &[Key::Char('z'), Key::Char('M')]);
    assert_eq!(show_ref(&folded).line_count(), 1, "everything is folded");
    let mut searching = folded;
    commit_search(&mut searching, "needle");

    let (stepped, _) = update(Msg::Key(Key::Char('n')), &searching);
    let show = show_ref(&stepped);
    let search = stepped.workspace.search.as_ref().expect("search");
    let matched = search.matches[search.cursor].line;
    assert!(
        matched == 3 || matched == 4,
        "the match is in the folded body: {matched}"
    );
    assert_eq!(show.line_count(), 6, "every enclosing region opened");
    assert_eq!(
        show.visible_row_for_source_line(matched),
        Some(matched - 1),
        "the matched line is now a real source row"
    );
    let row = show.visible_row_for_source_line(matched).expect("a row");
    let viewport = stepped.size.height as usize - 4;
    assert!(
        row >= show.body.scroll as usize && row < show.body.scroll as usize + viewport,
        "the matched row is scrolled into view: row {row}, scroll {}",
        show.body.scroll
    );
    assert!(
        !stepped
            .workspace
            .ranges_on_line(SearchPane::Source, matched)
            .is_empty(),
        "the match stays highlighted after the rows changed"
    );

    let closed = press(&stepped, &[Key::Char('z'), Key::Char('M')]);
    assert_eq!(
        show_ref(&closed).line_count(),
        1,
        "an explicit zM closes the auto-opened regions"
    );
}

#[cfg(feature = "diff")]
#[test]
fn stepping_to_a_match_moves_the_cursor_to_the_match_row() {
    let mut lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
    lines[24] = "the needle line".to_owned();
    let source = lines.join("\n") + "\n";
    let mut app = show_app(vec![source_outline("a.rs", &source)]);
    app.size.height = 8;
    focus_content(&mut app);
    commit_search(&mut app, "needle");

    let (stepped, _) = update(Msg::Key(Key::Char('n')), &app);
    let stepped = settle(stepped);
    let show = show_ref(&stepped);
    let search = stepped.workspace.search.as_ref().expect("search");
    let matched = search.matches[search.cursor].line;
    assert_eq!(matched, 25, "the needle is on line 25");
    assert_eq!(
        show.body.cursor_line, matched,
        "stepping moves the cursor onto the match"
    );
    assert_view_consistent(&stepped, 8);

    let advanced_app = press(&stepped, &[Key::Char('j')]);
    let advanced = show_ref(&advanced_app);
    assert_eq!(advanced.body.cursor_line, matched + 1);
    assert_view_consistent(&advanced_app, 8);
}

#[test]
fn the_folded_body_renders_the_signature_and_hides_the_body() {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    let text = buffer_text(&render(&app, 100, 20));

    assert!(text.contains("impl Widget {"), "{text}");
    assert!(text.contains("fn size(&self) -> usize {"), "{text}");
    assert!(text.contains("⋯ 3 lines"), "fold suffix missing: {text}");
    assert!(
        !text.contains("42"),
        "the hidden body must be absent: {text}"
    );
    assert!(
        !text.contains("let hidden"),
        "the hidden body must be absent: {text}"
    );
}

#[test]
fn a_dropped_declaration_renders_the_hidden_in_mode_tag() {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        false,
    )]);
    let text = buffer_text(&render(&app, 100, 20));

    assert!(
        text.contains("hidden in types"),
        "the dropped tag is missing: {text}"
    );
    assert!(
        !text.contains("42"),
        "the hidden body must be absent: {text}"
    );
}

#[test]
fn tests_mode_removes_dropped_declarations_entirely() {
    let app = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests);
    let show = show_ref(&app);

    for hidden in [1, 2, 3, 4, 9] {
        assert_eq!(
            show.visible_row_for_source_line(hidden),
            None,
            "line {hidden} must have no visible row: {:?}",
            show.visible_rows()
        );
    }
    assert!(
        !show
            .visible_rows()
            .iter()
            .any(|row| matches!(row, VisibleRow::Fold { dropped: true, .. })),
        "tests mode must not emit a `hidden in tests` marker row: {:?}",
        show.visible_rows()
    );

    assert_eq!(show.visible_row_for_source_line(5), Some(0));
    assert_eq!(show.visible_row_for_source_line(6), Some(1));
    assert_eq!(show.visible_row_for_source_line(7), Some(2));
    assert_eq!(
        show.visible_row_for_source_line(12),
        Some(5),
        "a line inside a collapsed retained test maps to its fold row"
    );
    assert_eq!(
        show.visible_rows()[5],
        VisibleRow::Fold {
            line: 11,
            summary_line: 12,
            region_index: 3,
            hidden: 3,
            dropped: false,
        }
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("mod tests {"), "{text}");
    assert!(text.contains("use super::*;"), "{text}");
    assert!(text.contains("fn checks() {"), "{text}");
    assert!(text.contains("fn top_level() {"), "{text}");
    assert!(
        !text.contains("pub enum Dropped"),
        "the dropped declaration leaked: {text}"
    );
    assert!(
        !text.contains("fn helper"),
        "the dropped helper leaked: {text}"
    );
    assert!(
        !text.contains("let hidden"),
        "the hidden body leaked: {text}"
    );
    assert!(
        !text.contains("assert!(true)"),
        "the hidden body leaked: {text}"
    );
    assert!(
        !text.contains("hidden in tests"),
        "a marker row leaked: {text}"
    );
}

#[test]
fn tests_mode_with_no_retained_declarations_renders_the_empty_state() {
    let app = show_app_mode(
        vec![dropped_container_outline("a.rs", NESTED_SOURCE)],
        ProjectionMode::Tests,
    );
    assert_eq!(
        show_ref(&app).line_count(),
        0,
        "a file with nothing retained has no rows"
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("No test declarations"), "{text}");
    assert!(text.contains("Nothing retained in this mode."), "{text}");
}

#[test]
fn tests_mode_selection_anchors_the_cursor_to_a_visible_row() {
    let mut app = show_app_mode(
        vec![tests_filter_outline(), source_outline("z.rs", "x\n")],
        ProjectionMode::Tests,
    );

    assert_eq!(
        show_ref(&app).body.cursor_line,
        5,
        "the fallback selection anchors the cursor"
    );
    assert_eq!(show_ref(&app).cursor_row(), Some(0), "the cursor is drawn");

    select_file(&mut app, RepoPath::new("z.rs").expect("valid path"));
    assert_eq!(show_ref(&app).body.cursor_line, 1);
    select_file(&mut app, RepoPath::new("a.rs").expect("valid path"));
    let mut app = settle(app);

    let show = show_ref(&app);
    assert_eq!(
        show.body.cursor_line, 5,
        "a tree selection anchors onto the first visible row"
    );
    assert_eq!(show.cursor_row(), Some(0), "the cursor is drawn");
    assert!(show.visible_row_for_source_line(5).is_some());

    focus_content(&mut app);
    let down = press(&app, &[Key::Char('j')]);
    assert_eq!(show_ref(&down).body.cursor_line, 6);
    assert_eq!(show_ref(&down).cursor_row(), Some(1));
    let up = press(&down, &[Key::Char('k')]);
    assert_eq!(show_ref(&up).body.cursor_line, 5);
    assert_eq!(show_ref(&up).cursor_row(), Some(0));
}

#[test]
fn visible_row_for_source_line_reports_gaps_in_tests_mode() {
    let app = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests);
    let show = show_ref(&app);

    assert_eq!(
        show.visible_row_for_source_line(1),
        None,
        "a line in a dropped declaration has no row"
    );
    assert_eq!(
        show.visible_row_for_source_line(9),
        None,
        "a dropped helper inside a retained container has no row"
    );
    assert_eq!(
        show.visible_row_for_source_line(16),
        None,
        "a blank line outside every retained region has no row"
    );
    assert_eq!(
        show.visible_row_for_source_line(6),
        Some(1),
        "a line in the retained container maps to its source row"
    );
    assert!(
        matches!(
            show.visible_row_for_source_line(13)
                .map(|row| &show.visible_rows()[row]),
            Some(VisibleRow::Fold { line: 11, .. })
        ),
        "a line inside a collapsed retained test maps to its fold row"
    );
}

#[test]
fn tests_mode_exclusion_survives_open_all() {
    let mut app = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests);
    focus_content(&mut app);
    let opened = press(&app, &[Key::Char('z'), Key::Char('R')]);
    let show = show_ref(&opened);
    for hidden in [1, 2, 3, 4, 9] {
        assert_eq!(
            show.visible_row_for_source_line(hidden),
            None,
            "line {hidden} must stay hidden after zR"
        );
    }
    assert!(
        !show
            .visible_rows()
            .iter()
            .any(|row| matches!(row, VisibleRow::Fold { dropped: true, .. }))
    );
}

#[test]
fn types_mode_keeps_marker_rows_for_dropped_declarations() {
    let app = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Types);
    let show = show_ref(&app);

    let dropped: Vec<usize> = show
        .visible_rows()
        .iter()
        .filter_map(|row| match row {
            VisibleRow::Fold {
                line,
                dropped: true,
                ..
            } => Some(*line),
            _ => None,
        })
        .collect();
    assert_eq!(
        dropped,
        vec![1, 9],
        "types mode still marks dropped declarations: {:?}",
        show.visible_rows()
    );

    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("pub enum Dropped"), "{text}");
    assert!(text.contains("hidden in types"), "{text}");
}

#[cfg(feature = "diff")]
#[test]
fn tests_mode_search_ignores_hidden_dropped_matches_and_reveals_retained_ones() {
    let mut app = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests);
    focus_content(&mut app);
    commit_search(&mut app, "hidden");
    let matches = &app.workspace.search.as_ref().expect("search").matches;
    assert_eq!(
        matches.len(),
        1,
        "only the visible `let hidden` line matches"
    );
    assert_eq!(matches[0].line, 13);

    let stepped = press(&app, &[Key::Char('n')]);
    let show = show_ref(&stepped);
    assert_eq!(show.body.cursor_line, 13, "the cursor reaches the match");
    assert!(
        show.visible_row_for_source_line(13).is_some(),
        "the match's line is now a visible source row"
    );

    let mut dropped = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests);
    commit_search(&mut dropped, "Dropped");
    assert!(
        dropped
            .workspace
            .search
            .as_ref()
            .expect("search")
            .matches
            .is_empty(),
        "a match inside a dropped declaration must not be counted"
    );
    let before = show_ref(&dropped).body.cursor_line;
    let stepped = press(&dropped, &[Key::Char('n')]);
    assert_eq!(
        show_ref(&stepped).body.cursor_line,
        before,
        "stepping does nothing when no visible match exists"
    );
}

#[test]
fn the_body_cursor_moves_and_stays_in_view() {
    let source: String = (1..=20).map(|n| format!("line {n}\n")).collect();
    let mut app = show_app(vec![source_outline("a.rs", &source)]);
    app.size.height = 6;
    focus_content(&mut app);
    assert_eq!(show_ref(&app).body.cursor_line, 1);

    let viewport = app.size.height as usize - 4;
    for _ in 0..5 {
        app = press(&app, &[Key::Char('j')]);
    }
    let show = show_ref(&app);
    let cursor = show.body.cursor_line;
    let row = show.visible_row_for_source_line(cursor).expect("a row");
    assert_eq!(cursor, 6, "j moves one visible row");
    assert!(show.body.scroll > 0, "the cursor stays in view");
    assert!(
        row >= show.body.scroll as usize && row < show.body.scroll as usize + viewport,
        "the cursor row is within the viewport"
    );

    let down = press(&app, &[Key::Char('G')]);
    assert_eq!(show_ref(&down).body.cursor_line, 20);
    assert!(show_ref(&down).body.scroll > 0, "G scrolls the last row in");

    let up = press(&down, &[Key::Char('g')]);
    assert_eq!(show_ref(&up).body.cursor_line, 1);
    assert_eq!(show_ref(&up).body.scroll, 0);

    let end = press(&up, &[Key::End]);
    assert_eq!(show_ref(&end).body.cursor_line, 20);
    assert_eq!(show_ref(&press(&end, &[Key::Home])).body.cursor_line, 1);
}

#[test]
fn za_toggles_the_fold_at_the_cursor() {
    let mut app = folder_app();
    focus_content(&mut app);
    assert_eq!(show_ref(&app).line_count(), 3);

    let app = press(&app, &[Key::Char('j')]);
    assert_eq!(show_ref(&app).body.cursor_line, 2);

    let expanded = press(&app, &[Key::Char('z'), Key::Char('a')]);
    assert_eq!(show_ref(&expanded).line_count(), 6, "the body rows appear");
    assert_eq!(show_ref(&expanded).body.cursor_line, 2);

    let collapsed = press(&expanded, &[Key::Char('z'), Key::Char('a')]);
    assert_eq!(
        show_ref(&collapsed).line_count(),
        3,
        "the fold closes again"
    );
}

#[test]
fn za_at_the_container_start_folds_the_container() {
    let mut app = folder_app();
    focus_content(&mut app);
    let toggled = press(&app, &[Key::Char('z'), Key::Enter]);
    assert_eq!(show_ref(&toggled).line_count(), 1);
    assert_eq!(show_ref(&toggled).body.cursor_line, 1);
}

#[test]
fn zc_closes_the_enclosing_container_from_a_line_inside_it() {
    let mut app = folder_app();
    focus_content(&mut app);
    let last = press(&app, &[Key::Char('G')]);
    assert_eq!(show_ref(&last).body.cursor_line, 6);

    let closed = press(&last, &[Key::Char('z'), Key::Char('c')]);
    assert_eq!(show_ref(&closed).line_count(), 1, "the container folds");
    assert_eq!(show_ref(&closed).body.cursor_line, 1, "the cursor snaps in");
}

#[test]
fn zc_closes_the_innermost_enclosing_region() {
    let mut app = folder_app();
    focus_content(&mut app);
    let open = press(&app, &[Key::Char('j'), Key::Char('z'), Key::Char('o')]);
    assert_eq!(show_ref(&open).line_count(), 6);
    let inside = press(&open, &[Key::Char('j')]);
    assert_eq!(show_ref(&inside).body.cursor_line, 3);

    let closed = press(&inside, &[Key::Char('z'), Key::Char('c')]);
    assert_eq!(show_ref(&closed).line_count(), 3, "only the child folds");
    assert_eq!(
        show_ref(&closed).visible_rows()[1],
        VisibleRow::Fold {
            line: 2,
            summary_line: 2,
            region_index: 1,
            hidden: 3,
            dropped: false,
        }
    );
}

#[test]
fn z_open_recursive_and_close_recursive_set_descendants() {
    let mut app = folder_app();
    focus_content(&mut app);
    let closed = press(&app, &[Key::Char('z'), Key::Char('C')]);
    assert_eq!(show_ref(&closed).line_count(), 1, "zC closes the subtree");

    let opened = press(&closed, &[Key::Char('z'), Key::Char('O')]);
    assert_eq!(
        show_ref(&opened).line_count(),
        6,
        "zO opens the region and its descendants"
    );
}

#[test]
fn z_open_all_and_close_all_set_every_region() {
    let mut app = folder_app();
    focus_content(&mut app);
    let closed = press(&app, &[Key::Char('z'), Key::Char('M')]);
    assert_eq!(show_ref(&closed).line_count(), 1, "zM closes everything");

    let opened = press(&closed, &[Key::Char('z'), Key::Char('R')]);
    assert_eq!(show_ref(&opened).line_count(), 6, "zR opens everything");
}

#[test]
fn shrinking_fold_actions_clamp_the_scroll_and_keep_the_cursor_in_view() {
    let focus = |height: u16| {
        let mut app = folder_app();
        app.size.height = height;
        focus_content(&mut app);
        app
    };
    for (name, keys) in [
        ("zc", [Key::Char('z'), Key::Char('c')].as_slice()),
        ("zC", [Key::Char('z'), Key::Char('C')].as_slice()),
        ("zM", [Key::Char('z'), Key::Char('M')].as_slice()),
    ] {
        let bottom = press(&focus(6), &[Key::Char('G')]);
        assert!(show_ref(&bottom).body.scroll > 0, "{name}: G scrolled");
        let folded = press(&bottom, keys);
        assert_eq!(
            show_ref(&folded).line_count(),
            1,
            "{name}: everything folds"
        );
        assert_view_consistent(&folded, 6);
    }

    let bottom = press(&focus(6), &[Key::Char('G')]);
    let opened = press(&bottom, &[Key::Char('z'), Key::Char('R')]);
    assert_eq!(show_ref(&opened).line_count(), 6, "zR opens everything");
    assert_view_consistent(&opened, 6);
}

#[test]
fn fold_all_after_a_bottom_jump_leaves_source_rows_to_draw() {
    let mut app = show_app(vec![multi_method_outline("a.rs", 6)]);
    app.size.height = 8;
    focus_content(&mut app);
    let bottom = press(&app, &[Key::Char('G')]);
    assert!(
        show_ref(&bottom).body.scroll > 0,
        "G scrolled the tall body"
    );

    let folded = press(&bottom, &[Key::Char('z'), Key::Char('M')]);
    let show = show_ref(&folded);
    assert_eq!(show.line_count(), 1, "everything folds to one row");
    assert_eq!(show.body.scroll, 0, "the scroll is clamped back to zero");
    assert_view_consistent(&folded, 8);

    let text = buffer_text(&render(&folded, 100, 8));
    assert!(
        text.contains("impl Widget {"),
        "the pane must not be blank: {text}"
    );
}

#[test]
fn a_fold_action_clamps_hscroll_into_range() {
    let mut app = folder_app();
    focus_content(&mut app);
    let mut wide = app;
    for _ in 0..128 {
        wide = press(&wide, &[Key::Char('l')]);
    }
    let before = show_ref(&wide).body.hscroll;
    assert!(before > 0, "the body panned right");
    assert_eq!(
        before as usize,
        show_ref(&wide).max_line_width(wide.workspace.selected.as_ref()) - 1,
        "panning stops at the widest visible row"
    );

    let opened = press(&wide, &[Key::Char('z'), Key::Char('R')]);
    assert!(
        show_ref(&opened).body.hscroll < before,
        "hscroll must shrink with the projection"
    );
    assert_view_consistent(&opened, wide.size.height);
}

#[test]
fn a_resize_scrolls_the_cursor_back_into_view() {
    let source: String = (1..=30).map(|n| format!("line {n}\n")).collect();
    let mut app = show_app(vec![source_outline("a.rs", &source)]);
    app.size.height = 20;
    focus_content(&mut app);
    let bottom = press(&app, &[Key::Char('G')]);
    assert_eq!(show_ref(&bottom).body.cursor_line, 30);

    let (resized, _) = update(
        Msg::Resize {
            width: 100,
            height: 6,
        },
        &bottom,
    );
    let resized = settle(resized);
    let show = show_ref(&resized);
    assert_eq!(
        show.body.cursor_line, 30,
        "the resize keeps the cursor on its line"
    );
    assert_view_consistent(&resized, 6);
}

#[test]
fn a_bare_enter_toggles_the_fold_in_the_body() {
    let mut app = folder_app();
    focus_content(&mut app);
    let app = press(&app, &[Key::Char('j')]);
    assert_eq!(show_ref(&app).body.cursor_line, 2);
    assert_eq!(show_ref(&app).line_count(), 3);

    let expanded = press(&app, &[Key::Enter]);
    assert_eq!(show_ref(&expanded).line_count(), 6, "Enter opens the fold");

    let collapsed = press(&expanded, &[Key::Enter]);
    assert_eq!(
        show_ref(&collapsed).line_count(),
        3,
        "Enter closes it again"
    );
}

#[test]
fn the_which_key_band_cancels_on_an_unrelated_key_and_on_esc() {
    let mut app = folder_app();
    focus_content(&mut app);

    let band = press(&app, &[Key::Char('z')]);
    assert_eq!(
        band.kernel.overlay,
        Some(WHICH_KEY_PANE),
        "pressing z opens the fold band"
    );
    assert_eq!(band.plugins.which_key.cursor, 0);
    assert_eq!(band.keymap.pending, Some('z'), "the prefix is pending");

    let cancelled = press(&band, &[Key::Char('x')]);
    assert_eq!(
        cancelled.kernel.overlay, None,
        "an unrelated key closes the band"
    );
    assert_eq!(cancelled.keymap.pending, None, "the prefix is cleared");
    assert_eq!(
        show_ref(&cancelled).body.cursor_line,
        1,
        "the unrelated key is swallowed"
    );

    let escaped = press(&band, &[Key::Esc]);
    assert_eq!(escaped.kernel.overlay, None, "Esc closes the band");
    assert_eq!(escaped.keymap.pending, None, "Esc clears the prefix");
    assert_eq!(show_ref(&escaped).body.cursor_line, 1);

    let after = press(&cancelled, &[Key::Char('j')]);
    assert_eq!(show_ref(&after).body.cursor_line, 2);
}

#[test]
fn a_mouse_event_closes_the_which_key_band() {
    let mut app = folder_app();
    focus_content(&mut app);
    let armed = press(&app, &[Key::Char('z')]);
    assert_eq!(armed.kernel.overlay, Some(WHICH_KEY_PANE));

    let (cleared, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &armed);
    assert!(cleared.kernel.overlay.is_none(), "the mouse drops the band");

    let moved = press(&cleared, &[Key::Char('j')]);
    assert_eq!(
        show_ref(&moved).body.cursor_line,
        2,
        "j was not swallowed by a stale band"
    );
}

#[test]
fn a_reload_closes_the_which_key_band() {
    let mut app = folder_app();
    focus_content(&mut app);
    let armed = press(&app, &[Key::Char('z')]);
    assert_eq!(armed.kernel.overlay, Some(WHICH_KEY_PANE));

    let (reloaded, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![nested_outline("a.rs", NESTED_SOURCE, (0, 5), (1, 4), true)].into()),
        }),
        &armed,
    );
    assert!(
        reloaded.kernel.overlay.is_none(),
        "a reload drops the pending band"
    );
}

#[test]
fn a_fold_override_survives_a_reload_and_a_selection_change() {
    let files = || {
        vec![
            nested_outline("a.rs", NESTED_SOURCE, (0, 5), (1, 4), true),
            source_outline("b.rs", "x\n"),
        ]
    };
    let mut app = show_app(files());
    focus_content(&mut app);
    let opened = press(&app, &[Key::Char('j'), Key::Char('z'), Key::Char('a')]);
    assert_eq!(show_ref(&opened).line_count(), 6);

    let (reloaded, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(files().into()),
        }),
        &opened,
    );
    let reloaded = settle(reloaded);
    assert_eq!(selected(&reloaded).as_deref(), Some("a.rs"));
    assert_eq!(
        show_ref(&reloaded).line_count(),
        6,
        "the override is sticky across a mode reload"
    );

    let mut tree = reloaded;
    tree.focus = Some(TREE_FOCUS);
    let down = press(&tree, &[Key::Char('j')]);
    assert_eq!(selected(&down).as_deref(), Some("b.rs"));
    let up = press(&down, &[Key::Char('k')]);
    assert_eq!(selected(&up).as_deref(), Some("a.rs"));
    assert_eq!(
        show_ref(&up).line_count(),
        6,
        "the override is sticky across a selection change"
    );
}

#[test]
fn palette_fold_entries_dispatch_the_same_effect_as_the_keys() {
    let app = folder_app();
    for (name, hint) in [
        ("fold.toggle", "za"),
        ("fold.open", "zo"),
        ("fold.close", "zc"),
        ("fold.open_recursive", "zO"),
        ("fold.close_recursive", "zC"),
        ("fold.open_all", "zR"),
        ("fold.close_all", "zM"),
    ] {
        let spec = command_spec(&app, SOURCE, name).expect("a fold command");
        assert_eq!(spec.hint, hint);
        assert!(
            has_palette_entry(&app, SOURCE, name),
            "missing palette entry for {hint}"
        );
    }

    let mut focused = app.clone();
    focus_content(&mut focused);
    let mut cmds = Vec::new();
    let mut via_palette = focused.clone();
    apply_command(&mut via_palette, SOURCE, "fold.close_all", &mut cmds);
    let via_keys = press(&focused, &[Key::Char('z'), Key::Char('M')]);
    assert_eq!(show_ref(&via_palette).line_count(), 1);
    assert_eq!(
        show_ref(&via_palette).line_count(),
        show_ref(&via_keys).line_count()
    );
}

#[test]
fn the_which_key_band_lists_the_fold_bindings() {
    let mut app = folder_app();
    focus_content(&mut app);
    let opened = press(&app, &[Key::Char('z')]);
    let buffer = render(&opened, 100, 20);
    let text = buffer_text(&buffer);
    assert!(
        text.contains("fold (z)"),
        "the band title is missing: {text}"
    );
    for label in [
        "Toggle fold",
        "Open fold",
        "Close fold",
        "Open fold recursively",
        "Close fold recursively",
        "Open all folds",
        "Close all folds",
    ] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    // The band sits above the status row, which keeps its own hints.
    let status = buffer_row(&buffer, buffer.area.height - 1);
    assert!(
        status.contains("Esc close"),
        "the status row must keep the band hints: {status}"
    );
}

#[test]
fn the_band_keeps_the_header_and_status_on_a_short_terminal() {
    let mut app = folder_app();
    focus_content(&mut app);
    let opened = press(&app, &[Key::Char('z')]);
    // 40 columns fits one entry column, so the seven bindings need more rows
    // than a short terminal has; the band must stay between header and status.
    let buffer = render(&opened, 40, 8);
    let text = buffer_text(&buffer);
    assert!(text.contains("fold (z)"), "the band still renders: {text}");
    assert!(
        text.contains("Esc close"),
        "the status row survives: {text}"
    );
    assert!(
        !buffer_row(&buffer, 0).contains("fold (z)"),
        "the band must not cover the header"
    );
}

#[test]
fn the_band_keys_select_and_run_a_binding() {
    let mut app = folder_app();
    focus_content(&mut app);

    // The first binding is highlighted when the band opens, so Enter runs
    // Toggle fold — the same effect as the `z a` keystrokes.
    let via_band = press(&app, &[Key::Char('j'), Key::Char('z'), Key::Enter]);
    let via_keys = press(&app, &[Key::Char('j'), Key::Char('z'), Key::Char('a')]);
    assert_eq!(
        show_ref(&via_band).line_count(),
        6,
        "the band's Enter ran the highlighted fold binding"
    );
    assert_eq!(
        show_ref(&via_band).line_count(),
        show_ref(&via_keys).line_count()
    );
    assert!(
        via_band.kernel.overlay.is_none(),
        "running a binding closes the band"
    );

    // Ctrl-J and Ctrl-K move the highlight without dispatching a binding.
    let opened = press(&app, &[Key::Char('z')]);
    assert_eq!(opened.kernel.overlay, Some(WHICH_KEY_PANE));
    assert_eq!(opened.plugins.which_key.cursor, 0);
    assert_eq!(opened.keymap.pending, Some('z'));
    let down = press(&opened, &[Key::CtrlJ]);
    assert_eq!(down.plugins.which_key.cursor, 1);
    let up = press(&down, &[Key::CtrlK]);
    assert_eq!(up.plugins.which_key.cursor, 0);
    let clamped = press(&up, &[Key::CtrlK]);
    assert_eq!(
        clamped.plugins.which_key.cursor, 0,
        "the highlight clamps at the first binding"
    );

    // The arrow keys move the same highlight.
    let arrows = press(&app, &[Key::Char('z'), Key::Down, Key::Down, Key::Up]);
    assert_eq!(arrows.plugins.which_key.cursor, 1);
}

#[test]
fn ctrl_j_and_ctrl_k_keep_their_j_k_alias_outside_the_band() {
    let mut app = folder_app();
    focus_content(&mut app);

    // Before the band existed Ctrl-J/Ctrl-K fell through to j/k; they still do
    // outside it.
    let down = press(&app, &[Key::CtrlJ]);
    assert_eq!(show_ref(&down).body.cursor_line, 2, "Ctrl-J is still `j`");
    let up = press(&down, &[Key::CtrlK]);
    assert_eq!(show_ref(&up).body.cursor_line, 1, "Ctrl-K is still `k`");
}

#[test]
fn the_band_bindings_match_their_palette_entries() {
    let app = folder_app();
    let (group, bindings) = with_cx(&app, |cx| {
        tui_core::keymap::prefix_group(cx.commands(), 'z', Some(SOURCE_FOCUS))
            .expect("the fold group")
    });

    for binding in &bindings {
        let hint = format!("{}{}", group.prefix, binding.key);
        assert_eq!(binding.hint, hint, "the hint must be the full keystroke");
        assert!(
            palette_entries(&app)
                .iter()
                .any(|(label, entry_hint)| *label == binding.label && *entry_hint == binding.hint),
            "no palette entry matches the band binding {hint}"
        );
    }
}

#[test]
fn q_under_a_pending_prefix_closes_the_band_without_quitting() {
    let mut app = folder_app();
    focus_content(&mut app);
    let band = press(&app, &[Key::Char('z')]);
    assert_eq!(band.kernel.overlay, Some(WHICH_KEY_PANE));
    assert_eq!(band.plugins.which_key.cursor, 0);
    assert_eq!(band.keymap.pending, Some('z'));

    let closed = press(&band, &[Key::Char('q')]);
    assert_eq!(closed.kernel.overlay, None, "q closes the band");
    assert!(
        !closed.chrome.quit,
        "q does not quit under a pending prefix"
    );
    assert_eq!(closed.keymap.pending, None, "the prefix is cleared");
    assert_eq!(
        show_ref(&closed).body.cursor_line,
        1,
        "q is swallowed, not typed or acted on"
    );
}

#[test]
fn a_pending_prefix_is_consumed_before_the_quit_check() {
    let mut app = folder_app();
    focus_content(&mut app);
    let band = press(&app, &[Key::Char('z')]);
    assert_eq!(band.keymap.pending, Some('z'));

    let swallowed = press(&band, &[Key::Char('q')]);
    assert!(
        !swallowed.chrome.quit,
        "the first q is consumed by the pending prefix"
    );

    let quit = press(&swallowed, &[Key::Char('q')]);
    assert!(quit.chrome.quit, "the next q quits");
}

#[test]
fn an_unhandled_key_falls_through_a_non_modal_overlay() {
    let mut app = two_files();
    // Force the band open with no pending prefix. It is non-modal, so a key it
    // does not handle falls through to the keymap.
    app.kernel.overlay = Some(WHICH_KEY_PANE);
    assert_eq!(app.keymap.pending, None);

    let (next, _) = update(Msg::Key(Key::Char('?')), &app);
    assert_eq!(next.kernel.overlay, Some(HELP_PANE));
}

#[test]
fn ctrl_c_quits_with_a_capturing_overlay_open() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
    assert_eq!(opened.kernel.overlay, Some(SEARCH_PANE));
    assert_eq!(opened.plugins.search.input.text, "");

    let (quit, _) = update(Msg::Key(Key::CtrlC), &opened);
    assert!(quit.chrome.quit, "Ctrl-C quits even from a search input");
}

#[test]
fn q_quits_from_help_and_from_the_mode_picker() {
    let app = two_files();

    let (help, _) = update(Msg::Key(Key::Char('?')), &app);
    assert_eq!(help.kernel.overlay, Some(HELP_PANE));
    let (quit_help, _) = update(Msg::Key(Key::Char('q')), &help);
    assert!(quit_help.chrome.quit, "q quits from help");

    let (mode, _) = update(Msg::Key(Key::Char('m')), &app);
    assert_eq!(mode.kernel.overlay, Some(MODES_PANE));
    let (quit_mode, _) = update(Msg::Key(Key::Char('q')), &mode);
    assert!(quit_mode.chrome.quit, "q quits from the mode picker");
}

#[test]
fn ctrl_j_and_ctrl_k_type_into_a_text_capturing_overlay() {
    let app = two_files();
    let (search, _) = update(Msg::Key(Key::Char('/')), &app);
    let (j, _) = update(Msg::Key(Key::CtrlJ), &search);
    let (jk, _) = update(Msg::Key(Key::CtrlK), &j);
    assert_eq!(jk.plugins.search.input.text, "jk");
    assert_eq!(jk.kernel.overlay, Some(SEARCH_PANE));
}

#[test]
fn esc_with_no_overlay_clears_the_diagnostic_and_the_search() {
    let mut app = show_app(vec![source_outline("a.rs", "alpha\nbeta\n")]);
    app.diagnostic = Some(Diagnostic::new("boom".to_owned()));
    app.workspace.search = Some(Search {
        needle: "alpha".to_owned(),
        matches: vec![SearchMatch {
            pane: SearchPane::Source,
            line: 1,
            start: 0,
            end: 5,
        }],
        cursor: 0,
    });

    let (cleared, _) = update(Msg::Key(Key::Esc), &app);
    assert!(cleared.diagnostic.is_none(), "Esc clears the diagnostic");
    assert!(cleared.workspace.search.is_none(), "Esc clears the search");
}

#[test]
fn the_band_reads_its_prefix_from_the_keymap() {
    let mut app = folder_app();
    focus_content(&mut app);
    let band = press(&app, &[Key::Char('z')]);
    assert_eq!(band.keymap.pending, Some('z'));
    assert_eq!(band.kernel.overlay, Some(WHICH_KEY_PANE));
    assert_eq!(band.plugins.which_key.cursor, 0);

    let text = buffer_text(&render(&band, 100, 20));
    assert!(
        text.contains("fold (z)"),
        "the band title reads the prefix: {text}"
    );
}

#[test]
fn closing_the_band_clears_the_pending_prefix() {
    // An unrelated key closes the band and drops the prefix.
    let mut app = folder_app();
    focus_content(&mut app);
    let band = press(&app, &[Key::Char('z')]);
    let closed = press(&band, &[Key::Char('x')]);
    assert_eq!(closed.keymap.pending, None, "an unrelated key clears it");

    // A mouse event closes the band and drops the prefix.
    let band = press(&app, &[Key::Char('z')]);
    let (mouse_closed, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &band);
    assert_eq!(mouse_closed.keymap.pending, None, "the mouse clears it");

    // A reload closes the band and drops the prefix.
    let band = press(&app, &[Key::Char('z')]);
    let (reloaded, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![nested_outline("a.rs", NESTED_SOURCE, (0, 5), (1, 4), true)].into()),
        }),
        &band,
    );
    assert_eq!(reloaded.keymap.pending, None, "a reload clears it");
}

#[cfg(feature = "diff")]
#[test]
fn the_palette_is_the_ranked_palette_subset_in_registry_order() {
    let show = folder_app();
    let range = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let snapshot = snapshot_range_app();
    let merge_base = merge_base_range_app();
    let commits = commits_app(3);

    // The kernel accessor is exactly the collected registry's `palette == true`
    // subset, sorted by (rank, registration order).
    for app in [&show, &range, &snapshot, &merge_base, &commits] {
        let expected = with_cx(app, |cx| {
            let mut specs: Vec<&tui_core::command::CommandSpec> =
                cx.commands().iter().filter(|spec| spec.palette).collect();
            specs.sort_by_key(|spec| spec.rank);
            specs
                .into_iter()
                .map(|spec| (spec.label, spec.hint))
                .collect::<Vec<_>>()
        });
        assert_eq!(palette_entries(app), expected);
    }

    let mut show_expected = vec![
        ("Switch types / signatures / tests", "m"),
        ("Change scope", "s"),
        ("Edit revision", "r"),
        ("Toggle fold", "za"),
        ("Open fold", "zo"),
        ("Close fold", "zc"),
        ("Open fold recursively", "zO"),
        ("Close fold recursively", "zC"),
        ("Open all folds", "zR"),
        ("Close all folds", "zM"),
    ];
    show_expected.extend(tail_palette_entries());
    assert_eq!(palette_entries(&show).len(), 20);
    assert_eq!(palette_entries(&show), show_expected);

    let mut range_expected = vec![
        ("Switch types / signatures / tests", "m"),
        ("Change scope", "s"),
        ("Edit base revision", "b"),
        ("Edit target revision", "t"),
        ("Switch to commits view", ""),
    ];
    range_expected.extend(tail_palette_entries());
    assert_eq!(palette_entries(&range).len(), 15);
    assert_eq!(palette_entries(&range), range_expected);

    let mut snapshot_expected = vec![
        ("Switch types / signatures / tests", "m"),
        ("Change scope", "s"),
        ("Edit base revision", "b"),
        ("Edit target revision", "t"),
    ];
    snapshot_expected.extend(tail_palette_entries());
    assert_eq!(palette_entries(&snapshot).len(), 14);
    assert_eq!(palette_entries(&snapshot), snapshot_expected);

    let mut merge_base_expected = vec![
        ("Switch types / signatures / tests", "m"),
        ("Change scope", "s"),
        ("Edit base revision", "b"),
        ("Edit target revision", "t"),
    ];
    merge_base_expected.extend(tail_palette_entries());
    assert_eq!(palette_entries(&merge_base).len(), 14);
    assert_eq!(palette_entries(&merge_base), merge_base_expected);

    let mut commits_expected = vec![
        ("Switch types / signatures / tests", "m"),
        ("Change scope", "s"),
        ("Switch to range view", ""),
    ];
    commits_expected.extend(tail_palette_entries());
    assert_eq!(palette_entries(&commits).len(), 13);
    assert_eq!(palette_entries(&commits), commits_expected);
}

#[cfg(feature = "diff")]
#[test]
fn command_lookup_reproduces_the_key_table() {
    let apps = [
        folder_app(),
        diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]),
        commits_app(3),
    ];
    for app in &apps {
        for spec in with_cx(app, |cx| cx.commands().to_vec()) {
            for binding in &spec.bindings {
                if let (
                    tui_core::command::KeySeq::Key(key),
                    tui_core::command::KeyScope::Anywhere,
                ) = (binding.seq, binding.scope)
                {
                    assert_eq!(
                        command_for(key, app),
                        Some(spec.id),
                        "{:?} must bind {key:?}",
                        spec.id
                    );
                }
            }
        }
        for key in [
            Key::Char('q'),
            Key::CtrlC,
            Key::Esc,
            Key::Enter,
            Key::CtrlJ,
            Key::CtrlK,
        ] {
            assert_eq!(command_for(key, app), None, "{key:?} must not be a command");
        }
    }
}

#[cfg(feature = "diff")]
#[test]
fn prefix_groups_are_the_fold_group_only_in_the_show_body() {
    let mut show = folder_app();
    focus_content(&mut show);
    let (group, bindings) = with_cx(&show, |cx| {
        tui_core::keymap::prefix_group(cx.commands(), 'z', Some(SOURCE_FOCUS))
            .expect("the fold group")
    });
    assert_eq!(group.prefix, 'z');
    assert_eq!(group.name, "fold");
    let keys: Vec<char> = bindings.iter().map(|b| b.key).collect();
    assert_eq!(keys, ['a', 'o', 'c', 'O', 'C', 'R', 'M']);
    let hints: Vec<&str> = bindings.iter().map(|b| b.hint).collect();
    assert_eq!(hints, ["za", "zo", "zc", "zO", "zC", "zR", "zM"]);

    // The prefix only parks while the show body holds focus.
    let prefix = |app: &App, focus: Option<FocusKey>| {
        with_cx(app, |cx| {
            tui_core::keymap::has_prefix(cx.commands(), 'z', focus)
        })
    };
    assert!(prefix(&show, show.kernel.focus));
    let tree = folder_app();
    assert!(!prefix(&tree, tree.kernel.focus));
    assert!(
        with_cx(&tree, |cx| tui_core::keymap::prefix_group(
            cx.commands(),
            'z',
            Some(TREE_FOCUS)
        ))
        .is_none(),
        "the tree focus has no prefix in scope"
    );
    assert!(
        with_cx(&show, |cx| tui_core::keymap::prefix_group(
            cx.commands(),
            'z',
            Some(SOURCE_FOCUS)
        ))
        .is_some()
    );
    for app in [
        diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]),
        commits_app(3),
    ] {
        assert!(!prefix(&app, app.kernel.focus));
        assert!(
            with_cx(&app, |cx| tui_core::keymap::prefix_group(
                cx.commands(),
                'z',
                Some(DIFF_BODY_FOCUS)
            ))
            .is_none()
        );
    }
}

#[cfg(feature = "diff")]
#[test]
fn every_palette_entry_dispatches_its_registered_binding() {
    for app in [
        folder_app(),
        diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]),
        commits_app(3),
    ] {
        let palette = palette_entries(&app);
        with_cx(&app, |cx| {
            let specs = cx.commands();
            for spec in specs {
                let listed = palette
                    .iter()
                    .any(|(label, hint)| *label == spec.label && *hint == spec.hint);
                assert_eq!(listed, spec.palette, "palette membership for {:?}", spec.id);

                for binding in &spec.bindings {
                    match binding.seq {
                        tui_core::command::KeySeq::Key(key)
                            if tui_core::command::scope_matches(binding.scope, app.focus) =>
                        {
                            assert_eq!(
                                tui_core::keymap::command_for_key(specs, key, app.focus),
                                Some(spec.id),
                            );
                        }
                        tui_core::command::KeySeq::Prefixed(_, _)
                            if !tui_core::command::scope_matches(binding.scope, app.focus) =>
                        {
                            // The fold prefix is scoped to the show body; it is
                            // not armed while another focus is active.
                        }
                        tui_core::command::KeySeq::Prefixed(prefix, second) => {
                            let band = cx.prefix_group(prefix).expect("a prefix group");
                            let binding = band
                                .bindings
                                .iter()
                                .find(|(key, _, _)| *key == second)
                                .expect("a band binding");
                            assert_eq!(binding.2, spec.id);
                        }
                        _ => {}
                    }
                }
            }
        });
    }
}

#[test]
fn the_focused_body_draws_the_cursor_row() {
    let mut app = folder_app();
    focus_content(&mut app);
    let theme = &app.chrome.theme;
    let cursor_bg = theme.color(theme.palette.surface_alt);

    let buffer = render(&app, 100, 20);
    let row = row_with(&buffer, "impl Widget {").expect("the cursor row");
    let x = cell_x_of(&buffer, row, "impl Widget {").expect("the source cell");
    assert_eq!(
        buffer.cell((x, row)).map(|cell| cell.bg),
        Some(cursor_bg),
        "the cursor row carries a distinct background"
    );

    let unfocused = folder_app();
    let buffer = render(&unfocused, 100, 20);
    let row = row_with(&buffer, "impl Widget {").expect("the cursor row");
    let x = cell_x_of(&buffer, row, "impl Widget {").expect("the source cell");
    assert_ne!(
        buffer.cell((x, row)).map(|cell| cell.bg),
        Some(cursor_bg),
        "an unfocused body draws no cursor"
    );
}

#[test]
fn the_tree_keeps_a_quiet_selected_row_when_the_body_has_focus() {
    use tui_core::render::theme::{Capability, Flavor};

    for flavor in [Flavor::Dark, Flavor::Light] {
        let mut app = two_files();
        app.chrome.theme = Theme::new(flavor, Capability::TrueColor);

        let focused = render(&app, 100, 20);
        let focused_row = (0..focused.area.height)
            .find(|&y| {
                focused
                    .cell((2, y))
                    .is_some_and(|cell| cell.symbol() == "▌")
            })
            .expect("selected file row");
        assert_eq!(focused.cell((2, focused_row)).unwrap().symbol(), "▌");

        focus_content(&mut app);
        let unfocused = render(&app, 100, 20);
        assert_eq!(unfocused.cell((2, focused_row)).unwrap().symbol(), "▏");
    }

    let mut app = two_files();
    app.chrome.theme = Theme::new(Flavor::Dark, Capability::NoColor);
    focus_content(&mut app);
    assert!(buffer_text(&render(&app, 100, 20)).contains("▏a.rs"));
}

#[test]
fn nerd_icons_render_and_leave_labels_intact() {
    let build = || {
        show_app(vec![
            source_outline("src/main.rs", "x\n"),
            source_outline("src/Main.elm", "y\n"),
        ])
    };
    let plain = build();
    let mut nerd = build();
    nerd.chrome.icons = Icons::new(IconStyle::Nerd);

    let plain_text = buffer_text(&render(&plain, 100, 20));
    let nerd_text = buffer_text(&render(&nerd, 100, 20));

    assert!(!plain_text.contains('\u{f07b}'), "no glyph by default");
    assert!(nerd_text.contains('\u{f07b}'), "folder glyph missing");
    assert!(nerd_text.contains('\u{e7a8}'), "rust glyph missing");
    assert!(nerd_text.contains('\u{e62c}'), "elm glyph missing");
    assert!(nerd_text.contains("main.rs"), "the label survives");
}

#[cfg(feature = "diff")]
#[test]
fn a_wide_terminal_shows_both_diff_sides_with_revision_labels() {
    let app = diff_app(vec![file_diff(
        "a.rs",
        Some("old line\n"),
        Some("new line\n"),
    )]);
    let text = buffer_text(&render(&app, 120, 20));
    assert!(text.contains("HEAD~1"), "old label missing: {text}");
    assert!(text.contains("HEAD"), "new label missing: {text}");
    assert!(text.contains("old line"), "old side missing: {text}");
    assert!(text.contains("new line"), "new side missing: {text}");
}

#[cfg(feature = "diff")]
#[test]
fn a_narrow_diff_stacks_old_over_new_when_focused() {
    let mut app = diff_app(vec![file_diff(
        "a.rs",
        Some("old line\n"),
        Some("new line\n"),
    )]);
    focus_content(&mut app);
    let text = buffer_text(&render(&app, 60, 20));
    assert!(text.contains("old line"), "{text}");
    assert!(text.contains("new line"), "{text}");
}

#[test]
fn a_narrow_terminal_shows_only_the_focused_pane() {
    let app = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    let tree = buffer_text(&render(&app, 60, 20));
    assert!(tree.contains("Files"));
    assert!(!tree.contains("pub fn a();"), "body must be hidden: {tree}");

    let mut body = app;
    focus_content(&mut body);
    let body = buffer_text(&render(&body, 60, 20));
    assert!(!body.contains("Files"), "tree must be hidden: {body}");
    assert!(body.contains("pub fn a();"));
}

#[test]
fn a_tiny_terminal_shows_a_message_instead_of_a_frame() {
    let app = two_files();
    let text = buffer_text(&render(&app, 30, 5));
    assert!(text.contains("terminal too small"), "{text}");
}

#[test]
fn an_empty_result_renders_a_clear_empty_state() {
    let app = show_app(Vec::new());
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("No files"), "{text}");
    assert!(text.contains("No source files in this scope."), "{text}");
}

#[cfg(feature = "diff")]
#[test]
fn a_diff_with_no_rows_renders_a_clear_empty_state() {
    let app = diff_app(Vec::new());
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("No changes"), "{text}");
    assert!(text.contains("Body-only edits are omitted."), "{text}");
}

#[test]
fn a_long_line_is_clipped_and_never_overwrites_the_tree() {
    let long = "x".repeat(300);
    let app = show_app(vec![source_outline("a.rs", &format!("{long}\n"))]);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Files"), "the tree must survive: {text}");
    assert!(!text.contains(&long), "the long line must be clipped");
    assert!(text.contains(&"x".repeat(40)), "the visible prefix remains");
}

#[test]
fn responsive_boundaries_pick_the_right_layout() {
    let app = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    let wide = buffer_text(&render(&app, 80, 20));
    assert!(wide.contains("Files") && wide.contains("pub fn a();"));

    let narrow = buffer_text(&render(&app, 79, 20));
    assert!(narrow.contains("Files") && !narrow.contains("pub fn a();"));

    assert!(!buffer_text(&render(&app, 40, 8)).contains("terminal too small"));
    assert!(buffer_text(&render(&app, 39, 8)).contains("terminal too small"));
    assert!(buffer_text(&render(&app, 100, 7)).contains("terminal too small"));
}

#[cfg(feature = "diff")]
#[test]
fn loaded_panes_lists_the_tree_then_the_view_bodies() {
    let ids = |app: &App| -> Vec<crate::plugin::PaneId> {
        panes(app).iter().map(|spec| spec.id).collect()
    };

    let show = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    assert_eq!(ids(&show), vec![TREE_PANE, SOURCE_PANE]);

    let range = diff_app(vec![file_diff("a.rs", Some("x\n"), Some("y\n"))]);
    assert_eq!(ids(&range), vec![TREE_PANE, OLD_PANE, NEW_PANE]);

    let commits = commits_app(3);
    assert_eq!(
        ids(&commits),
        vec![TREE_PANE, OLD_PANE, NEW_PANE, COMMITS_PANE]
    );
}

#[cfg(feature = "diff")]
#[test]
fn the_tree_plugin_owns_its_pane_and_the_views_do_not() {
    for app in [
        show_app(vec![source_outline("a.rs", "pub fn a();\n")]),
        diff_app(vec![file_diff("a.rs", Some("x\n"), Some("y\n"))]),
        commits_app(3),
    ] {
        let union = panes(&app);
        let tree = union.iter().filter(|spec| spec.id == TREE_PANE).count();
        assert_eq!(tree, 1, "the union owns TREE_PANE exactly once");

        let specs = tui_core::app::Specs::panes_only(&app);
        let overlay = app.kernel.overlay;
        let busy = app.plugins.busy();
        let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);
        let views: Vec<tui_core::layout::PaneSpec> = app
            .plugins
            .source
            .panes(&cx)
            .into_iter()
            .chain(app.plugins.diff.panes(&cx))
            .collect();
        assert!(
            views.iter().all(|spec| spec.id != TREE_PANE),
            "the views no longer declare the tree pane: {views:?}"
        );
        assert!(
            views.iter().all(|spec| spec.focus != TREE_FOCUS),
            "the views no longer declare the tree focus"
        );
    }
}

#[cfg(feature = "diff")]
#[test]
fn the_source_and_diff_sides_share_one_focus_stop() {
    let show = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    assert_eq!(focus_ring(&panes(&show)), vec![TREE_FOCUS, SOURCE_FOCUS]);

    // The range's two sides share one body focus key, so the ring has two stops.
    let range = diff_app(vec![file_diff("a.rs", Some("x\n"), Some("y\n"))]);
    assert_eq!(
        focus_ring(&panes(&range)),
        vec![TREE_FOCUS, DIFF_BODY_FOCUS]
    );
    let (step, _) = update(Msg::Key(Key::Tab), &range);
    assert_eq!(diff_focus_of(&step), Some(DIFF_BODY_FOCUS));
    let (step, _) = update(Msg::Key(Key::Tab), &step);
    assert_eq!(diff_focus_of(&step), Some(TREE_FOCUS));

    // The commits band comes first, then the navigator, then the shared body.
    let commits = commits_app(3);
    assert_eq!(
        focus_ring(&panes(&commits)),
        vec![DIFF_BAND_FOCUS, TREE_FOCUS, DIFF_BODY_FOCUS]
    );
}

#[cfg(feature = "diff")]
#[test]
fn body_visible_truth_table() {
    let at = |app: &mut App, width: u16, focus: FocusKey| {
        app.size = Size { width, height: 8 };
        app.focus = Some(focus);
    };

    let mut show = show_app(vec![source_outline("a.rs", "pub fn a();\n")]);
    at(&mut show, 40, TREE_FOCUS);
    assert!(!body_is_visible(&show), "narrow tree focus hides the body");
    at(&mut show, 40, SOURCE_FOCUS);
    assert!(body_is_visible(&show), "narrow body focus shows the body");
    at(&mut show, 80, TREE_FOCUS);
    assert!(body_is_visible(&show), "wide always shows the body");
    at(&mut show, 80, SOURCE_FOCUS);
    assert!(body_is_visible(&show));

    let mut commits = commits_app(3);
    at(&mut commits, 40, DIFF_BAND_FOCUS);
    assert!(
        body_is_visible(&commits),
        "the band focus keeps the body visible"
    );
    at(&mut commits, 80, DIFF_BAND_FOCUS);
    assert!(body_is_visible(&commits));

    at(&mut show, 39, SOURCE_FOCUS);
    assert!(!body_is_visible(&show), "too narrow is never visible");
    show.size = Size {
        width: 80,
        height: 7,
    };
    assert!(!body_is_visible(&show), "too short is never visible");
}

#[test]
fn horizontal_scroll_shifts_the_visible_columns() {
    let mut app = show_app(vec![source_outline("a.rs", "abcdef\n")]);
    focus_content(&mut app);
    assert!(buffer_text(&render(&app, 100, 20)).contains("abcdef"));

    let (scrolled, _) = update(Msg::Key(Key::Char('l')), &app);
    let text = buffer_text(&render(&scrolled, 100, 20));
    assert!(text.contains("bcdef"), "the tail must remain: {text}");
    assert!(
        !text.contains("abcdef"),
        "the first column must scroll away"
    );
}

#[test]
fn the_show_pane_carries_syntax_colors() {
    if !tui_core::render::theme::colors_enabled() {
        return;
    }
    let app = show_app(vec![source_outline("a.rs", "pub struct User;\n")]);
    let buffer = render(&app, 100, 20);
    let colored = (0..buffer.area.height).any(|y| {
        (0..buffer.area.width).any(|x| {
            buffer
                .cell((x, y))
                .is_some_and(|cell| cell.fg != Color::Reset)
        })
    });
    assert!(colored, "expected a syntax foreground in the show pane");
}

#[test]
fn non_utf8_path_components_render_escaped() {
    let path =
        SupportedPath::new(RepoPath::new(b"src/\xFF/lib.rs".as_slice()).expect("valid path"))
            .expect("supported path");
    let outline =
        assemble_outline(&path, vec![item("x\n")], vec![item("x\n")]).expect("valid fixture");
    let file = SourceOutline {
        outline,
        source: "x\n".to_owned(),
    };
    let app = show_app(vec![file]);
    let labels: Vec<String> = app
        .plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.clone())
        .collect();
    assert!(labels.iter().any(|label| label == "src"), "{labels:?}");
    assert!(
        labels.iter().any(|label| label.contains("\\xFF")),
        "{labels:?}"
    );
}

#[cfg(feature = "diff")]
#[test]
fn a_long_diff_line_is_wrapped_inside_its_pane() {
    let long = "y".repeat(300);
    let app = diff_app(vec![file_diff(
        "a.rs",
        Some("x\n"),
        Some(&format!("{long}\n")),
    )]);
    let text = buffer_text(&render(&app, 120, 20));
    assert!(text.contains('…'), "expected continuation markers: {text}");
    assert!(!text.contains(&long), "the long line must be wrapped");
}

#[test]
fn the_help_overlay_lists_the_keys() {
    let mut app = two_files();
    app.kernel.overlay = Some(HELP_PANE);
    let text = buffer_text(&render(&app, 100, 32));
    assert!(text.contains("help"), "{text}");
    assert!(text.contains("quit"), "{text}");
    assert!(text.contains("Fold"), "the fold section is missing: {text}");
    for key in ["Enter / za", "zo / zc", "zO / zC", "zR / zM"] {
        assert!(text.contains(key), "missing fold key {key}: {text}");
    }
    assert!(
        text.contains("wheel"),
        "the popup must fit every line: {text}"
    );
}

#[cfg(feature = "diff")]
#[test]
fn a_changed_diff_row_gets_delta_backgrounds() {
    if !tui_core::render::theme::colors_enabled() {
        return;
    }
    let app = diff_app(vec![file_diff(
        "a.rs",
        Some("pub id: u32;\n"),
        Some("pub id: u64;\n"),
    )]);
    let buffer = render(&app, 120, 20);
    let theme = &app.chrome.theme;
    let delete_bg = theme.color(theme.palette.del_bg);
    let add_bg = theme.color(theme.palette.add_bg);
    let add_emph = theme.color(theme.palette.add_emph);
    let del_emph = theme.color(theme.palette.del_emph);
    let mut delete = false;
    let mut add = false;
    let mut emphasis = false;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                delete |= cell.bg == delete_bg;
                add |= cell.bg == add_bg;
                emphasis |= cell.bg == add_emph || cell.bg == del_emph;
            }
        }
    }
    assert!(delete, "expected a removed-line background");
    assert!(add, "expected an added-line background");
    assert!(emphasis, "expected intra-line emphasis");
}

#[test]
fn the_canvas_is_opaque_in_every_flavor() {
    use tui_core::render::theme::{Capability, Flavor};

    for flavor in [Flavor::Dark, Flavor::Light] {
        for overlay in [None, Some(HELP_PANE)] {
            let mut app = two_files();
            app.chrome.theme = Theme::new(flavor, Capability::TrueColor);
            app.kernel.overlay = overlay;
            let buffer = render(&app, 100, 20);
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    let cell = buffer.cell((x, y)).expect("cell");
                    assert_ne!(
                        cell.bg,
                        Color::Reset,
                        "the terminal background leaked at ({x}, {y}) with {flavor:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_canvas_takes_the_light_palette_background() {
    use tui_core::render::theme::{Capability, Flavor};

    let mut app = two_files();
    let light = Theme::new(Flavor::Light, Capability::TrueColor);
    let bg = light.color(light.palette.bg);
    app.chrome.theme = light;
    let buffer = render(&app, 100, 20);
    let mut count = 0;
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if buffer.cell((x, y)).is_some_and(|cell| cell.bg == bg) {
                count += 1;
            }
        }
    }
    assert!(count > 0, "expected the light background somewhere");
}

#[test]
fn clip_line_slices_by_display_width() {
    use tui_core::render::text::clip_line;
    assert_eq!(clip_line("abcdef", 2, 3), "cde");
    assert_eq!(clip_line("abcdef", 0, 0), "");
    assert_eq!(clip_line("abcdef", 10, 3), "");
    assert_eq!(clip_line("a\tb", 0, 10), "a    b");
    assert_eq!(clip_line("日本", 0, 3), "日");
    assert_eq!(clip_line("日本", 1, 4), "本");
}

#[test]
fn window_offset_keeps_the_cursor_visible() {
    use tui_core::layout::window_offset;
    assert_eq!(window_offset(0, 100, 10), 0);
    assert_eq!(window_offset(9, 100, 10), 0);
    assert_eq!(window_offset(10, 100, 10), 1);
    assert_eq!(window_offset(0, 3, 10), 0);
    assert_eq!(window_offset(5, 100, 0), 0);
}

/// Apply a request through the kernel path and return its effects.
fn run_request(app: &mut App, request: Request) -> Vec<PluginsCmd> {
    let mut cmds = Vec::new();
    apply_request(app, request, &mut cmds);
    cmds
}

fn search_of(app: &App) -> &Search {
    app.workspace.search.as_ref().expect("a committed search")
}

#[test]
fn collect_matches_is_case_insensitive_and_non_overlapping() {
    // Case-insensitive per character: the uppercase `A` matches.
    let mut insensitive = Vec::new();
    collect_matches("aAa", "a", SearchPane::Source, &mut insensitive);
    assert_eq!(
        insensitive
            .iter()
            .map(|m| (m.line, m.start, m.end))
            .collect::<Vec<_>>(),
        vec![(1, 0, 1), (1, 1, 2), (1, 2, 3)],
        "the uppercase character matches and the last end is the line length"
    );

    // Non-overlapping: a match consumes its width before the next search.
    let mut gaps = Vec::new();
    collect_matches("aXa", "a", SearchPane::Source, &mut gaps);
    assert_eq!(
        gaps.iter()
            .map(|m| (m.line, m.start, m.end))
            .collect::<Vec<_>>(),
        vec![(1, 0, 1), (1, 2, 3)],
        "the non-matching character is skipped; end is the line length"
    );
}

#[test]
fn search_ranges_marks_only_the_global_cursor_match() {
    let mut app = show_app(vec![source_outline("a.rs", "x x\n")]);
    app.workspace.search = Some(Search {
        needle: "x".to_owned(),
        matches: vec![
            SearchMatch {
                pane: SearchPane::Source,
                line: 1,
                start: 0,
                end: 1,
            },
            SearchMatch {
                pane: SearchPane::Source,
                line: 1,
                start: 2,
                end: 3,
            },
        ],
        cursor: 1,
    });
    assert_eq!(
        app.workspace.ranges_on_line(SearchPane::Source, 1),
        vec![(0, 1, false), (2, 3, true)],
        "the match the cursor names is the current one"
    );

    // A cursor on another pane leaves nothing current on this pane.
    app.workspace.search = Some(Search {
        needle: "x".to_owned(),
        matches: vec![
            SearchMatch {
                pane: SearchPane::Old,
                line: 1,
                start: 0,
                end: 1,
            },
            SearchMatch {
                pane: SearchPane::Source,
                line: 1,
                start: 2,
                end: 3,
            },
        ],
        cursor: 0,
    });
    assert_eq!(
        app.workspace.ranges_on_line(SearchPane::Source, 1),
        vec![(2, 3, false)],
        "no range is current while the cursor is on another pane"
    );
}

#[cfg(feature = "diff")]
#[test]
fn search_matches_tags_the_owning_pane() {
    let show = show_app(vec![source_outline("a.rs", "needle\n")]);
    assert!(
        show_ref(&show)
            .search_matches("needle", &show.workspace)
            .iter()
            .all(|matched| matched.pane == SearchPane::Source),
        "a show match belongs to the source pane"
    );

    let added = diff_app(vec![file_diff("a.rs", None, Some("needle\n"))]);
    let matches = diff_ref(&added).search_matches("needle", &added.workspace);
    assert!(!matches.is_empty());
    assert!(
        matches
            .iter()
            .all(|matched| matched.pane == SearchPane::New)
    );

    let deleted = diff_app(vec![file_diff("a.rs", Some("needle\n"), None)]);
    assert!(
        diff_ref(&deleted)
            .search_matches("needle", &deleted.workspace)
            .iter()
            .all(|matched| matched.pane == SearchPane::Old)
    );

    let modified = diff_app(vec![file_diff("a.rs", Some("needle\n"), Some("needle\n"))]);
    let matches = diff_ref(&modified).search_matches("needle", &modified.workspace);
    assert_eq!(matches.len(), 2, "both sides match a modified file");
    assert_eq!(matches[0].pane, SearchPane::Old, "the old side comes first");
    assert_eq!(matches[1].pane, SearchPane::New);
}

#[test]
fn settle_resyncs_the_workspace_search_and_clamps_the_cursor() {
    let mut app = show_app(vec![
        source_outline("a.rs", "alpha\nalpha\n"),
        source_outline("b.rs", "alpha\n"),
    ]);
    app.workspace.search = Some(Search {
        needle: "alpha".to_owned(),
        matches: vec![
            SearchMatch {
                pane: SearchPane::Source,
                line: 1,
                start: 0,
                end: 5,
            },
            SearchMatch {
                pane: SearchPane::Source,
                line: 2,
                start: 0,
                end: 5,
            },
        ],
        cursor: 1,
    });
    app.workspace.selected = Some(RepoPath::new("b.rs").unwrap());

    let app = settle(app);
    let search = search_of(&app);
    assert_eq!(search.matches.len(), 1, "recomputed for the new file");
    assert_eq!(search.cursor, 0, "the cursor is clamped into range");
}

#[test]
fn the_shell_advances_the_search_cursor_with_wrap_around() {
    let app = show_app(vec![source_outline("a.rs", "a\nb\na\n")]);
    let app = press(&app, &[Key::Char('/'), Key::Char('a'), Key::Enter]);
    assert_eq!(search_of(&app).matches.len(), 2);
    assert_eq!(search_of(&app).cursor, 0);

    let forward = press(&app, &[Key::Char('n')]);
    assert_eq!(search_of(&forward).cursor, 1);
    let wrapped = press(&forward, &[Key::Char('n')]);
    assert_eq!(
        search_of(&wrapped).cursor,
        0,
        "advancing wraps to the first"
    );

    let back = press(&wrapped, &[Key::Char('N')]);
    assert_eq!(search_of(&back).cursor, 1, "retreating wraps to the last");
}

#[cfg(feature = "diff")]
#[test]
fn commit_search_reveals_the_current_match_on_the_diff_pane() {
    let target: String = (1..=40)
        .map(|n| {
            if n == 40 {
                "needle\n".to_owned()
            } else {
                format!("line {n}\n")
            }
        })
        .collect();
    let mut app = diff_app(vec![file_diff("a.rs", None, Some(&target))]);
    commit_search(&mut app, "needle");
    assert_eq!(search_of(&app).matches.len(), 1);
    assert_eq!(
        diff_ref(&app).body_scroll(),
        0,
        "committing does not scroll"
    );

    run_request(&mut app, Request::CommitSearch);
    assert!(
        diff_ref(&app).body_scroll() > 0,
        "committing reveals the current match"
    );
}

#[cfg(feature = "diff")]
#[test]
fn query_loaded_commits_the_workspace_query() {
    let app = two_files();
    assert_eq!(app.workspace.query.mode, ProjectionMode::Types);

    let (committed, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![source_outline("a.rs", "a\n")].into()),
        }),
        &app,
    );
    assert_eq!(
        committed.workspace.query.mode,
        ProjectionMode::Signatures,
        "a successful load commits its mode"
    );

    let error = EngineError::Selection(base::SelectionError::UnknownArea {
        name: "x".to_owned(),
    });
    let (failed, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Tests,
                ..show_request()
            },
            result: Err(Box::new(error)),
        }),
        &committed,
    );
    assert_eq!(
        failed.workspace.query.mode,
        ProjectionMode::Signatures,
        "a failed load leaves the committed query alone"
    );
}

#[test]
fn a_scope_requery_keeps_the_loaded_mode() {
    let app = two_files();
    let (mut loaded, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![source_outline("a.rs", "a\n"), source_outline("b.rs", "b\n")].into()),
        }),
        &app,
    );
    assert_eq!(loaded.workspace.query.mode, ProjectionMode::Signatures);

    let selection = Selection::new(vec![SelectionGroup::Path(RepoPath::new("a.rs").unwrap())]);
    let cmds = run_request(
        &mut loaded,
        Request::Requery(QueryChange::Scope(selection.clone())),
    );
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            mode: ProjectionMode::Signatures,
            selection,
            ..show_request()
        }))],
        "the scope change reloads with the loaded mode"
    );
}

#[cfg(feature = "diff")]
#[test]
fn a_mode_requery_keeps_the_scope() {
    let selection = Selection::new(vec![SelectionGroup::Path(RepoPath::new("b.rs").unwrap())]);
    let app = base_app(LoadRequest::Show {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: selection.clone(),
    });
    let (mut loaded, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                revision: "HEAD".to_owned(),
                mode: ProjectionMode::Types,
                selection: selection.clone(),
            },
            result: Ok(vec![source_outline("a.rs", "a\n"), source_outline("b.rs", "b\n")].into()),
        }),
        &app,
    );
    assert_eq!(loaded.workspace.query.scope.selection, selection);

    let cmds = run_request(
        &mut loaded,
        Request::Requery(QueryChange::Mode(ProjectionMode::Tests)),
    );
    assert_eq!(
        cmds,
        vec![PluginsCmd::Source(crate::source::Cmd::Load(ShowRequest {
            revision: "HEAD".to_owned(),
            mode: ProjectionMode::Tests,
            selection,
        }))],
        "the mode change reloads with the committed scope"
    );
}

#[cfg(feature = "diff")]
#[test]
fn search_survives_a_mode_reload_and_resyncs() {
    let mut app = show_app(vec![source_outline("a.rs", "alpha\n")]);
    commit_search(&mut app, "alpha");
    assert_eq!(search_of(&app).matches[0].line, 1);

    let (reloaded, _) = update(
        source_msg(SourceMsg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![source_outline("a.rs", "beta\nalpha\n")].into()),
        }),
        &app,
    );
    assert!(reloaded.workspace.search.is_some(), "the search survives");

    let reloaded = settle(reloaded);
    let search = search_of(&reloaded);
    assert_eq!(search.needle, "alpha");
    assert_eq!(search.matches.len(), 1);
    assert_eq!(
        search.matches[0].line, 2,
        "the search re-synced against the reloaded file"
    );
}

#[cfg(feature = "diff")]
#[test]
fn set_search_refreshes_visible_rows_before_matching() {
    let mut app = show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests);
    focus_content(&mut app);
    commit_search(&mut app, "hidden");

    let search = search_of(&app);
    assert_eq!(
        search.matches.len(),
        1,
        "the search sees only the visible rows"
    );
    assert_eq!(search.matches[0].line, 13);
}

#[test]
fn tab_cycles_tree_and_source_body() {
    let app = two_files();
    assert_eq!(app.focus, Some(TREE_FOCUS));

    let (step, _) = update(Msg::Key(Key::Tab), &app);
    assert_eq!(step.focus, Some(SOURCE_FOCUS));
    let (step, _) = update(Msg::Key(Key::Tab), &step);
    assert_eq!(
        step.focus,
        Some(TREE_FOCUS),
        "Tab from the body wraps to the tree"
    );
}

#[test]
fn shift_tab_cycles_from_the_tree() {
    let app = two_files();
    let (step, _) = update(Msg::Key(Key::BackTab), &app);
    assert_eq!(
        step.focus,
        Some(SOURCE_FOCUS),
        "Shift-Tab from the tree wraps to the body"
    );
    let (step, _) = update(Msg::Key(Key::BackTab), &step);
    assert_eq!(step.focus, Some(TREE_FOCUS));
}

#[cfg(feature = "diff")]
#[test]
fn the_focus_ring_wraps_in_the_commits_view() {
    let app = commits_app(3);
    assert_eq!(app.focus, Some(DIFF_BAND_FOCUS));

    let (step, _) = update(Msg::Key(Key::Tab), &app);
    assert_eq!(step.focus, Some(TREE_FOCUS));
    let (step, _) = update(Msg::Key(Key::Tab), &step);
    assert_eq!(step.focus, Some(DIFF_BODY_FOCUS));
    let (step, _) = update(Msg::Key(Key::Tab), &step);
    assert_eq!(step.focus, Some(DIFF_BAND_FOCUS), "the ring wraps");

    let (back, _) = update(Msg::Key(Key::BackTab), &app);
    assert_eq!(
        back.focus,
        Some(DIFF_BODY_FOCUS),
        "backward wraps to the body"
    );
}

#[cfg(feature = "diff")]
#[test]
fn focus_defaults_to_the_navigator() {
    let show = two_files();
    assert_eq!(show.focus, Some(TREE_FOCUS));
    assert_eq!(focus_default(&panes(&show)), Some(TREE_FOCUS));

    let range = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    assert_eq!(range.focus, Some(TREE_FOCUS));

    // With no navigator, the first body pane is the default.
    let body_only = [tui_core::layout::PaneSpec {
        id: SOURCE_PANE,
        placement: tui_core::layout::Placement::Body,
        focus: SOURCE_FOCUS,
    }];
    assert_eq!(focus_default(&body_only), Some(SOURCE_FOCUS));
    assert_eq!(focus_default(&[]), None);
}

#[cfg(feature = "diff")]
#[test]
fn focus_normalises_when_the_band_disappears() {
    let app = commits_app(3);
    assert_eq!(app.focus, Some(DIFF_BAND_FOCUS));

    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request: diff_request(),
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into()),
        }),
        &app,
    );
    assert!(
        !panes(&next)
            .iter()
            .any(|spec| spec.focus == DIFF_BAND_FOCUS),
        "the band is gone"
    );
    assert_eq!(
        next.focus,
        Some(TREE_FOCUS),
        "focus falls back to the navigator"
    );
}

#[cfg(feature = "diff")]
#[test]
fn mouse_focuses_the_hit_pane_for_every_event_kind() {
    let mut app = commits_app(3);
    app.size = Size {
        width: 100,
        height: 20,
    };

    for mouse in [click(2, 3), scroll(2, 3, false), scroll(2, 3, true)] {
        let mut start = app.clone();
        start.focus = Some(SOURCE_FOCUS);
        let (next, _) = update(Msg::Mouse(mouse), &start);
        assert_eq!(next.focus, Some(TREE_FOCUS), "tree event {mouse:?}");
    }

    for mouse in [click(50, 3), scroll(50, 3, false), scroll(50, 3, true)] {
        let (next, _) = update(Msg::Mouse(mouse), &app);
        assert_eq!(next.focus, Some(DIFF_BODY_FOCUS), "body event {mouse:?}");
    }

    for mouse in [click(50, 12), scroll(50, 12, false), scroll(50, 12, true)] {
        let mut start = app.clone();
        start.focus = Some(TREE_FOCUS);
        let (next, _) = update(Msg::Mouse(mouse), &start);
        assert_eq!(next.focus, Some(DIFF_BAND_FOCUS), "band event {mouse:?}");
    }
}

#[cfg(feature = "diff")]
#[test]
fn a_band_click_outside_the_inner_rect_only_moves_focus() {
    let mut app = commits_app(3);
    app.size = Size {
        width: 100,
        height: 20,
    };
    app.focus = Some(TREE_FOCUS);
    let cursor = commits_ref(&app).cursor();

    // Row 10 is the band's top border: inside the outer rect, not the inner.
    let (next, _) = update(Msg::Mouse(click(50, 10)), &app);
    assert_eq!(next.focus, Some(DIFF_BAND_FOCUS));
    assert_eq!(
        commits_ref(&next).cursor(),
        cursor,
        "a border click does not select a commit"
    );
}

// -- the tree plugin ---------------------------------------------------------

#[test]
fn the_tree_input_is_pane_scoped() {
    let mut app = two_files();
    let specs = tui_core::app::Specs::panes_only(&app);
    let overlay = app.kernel.overlay;
    let busy = app.plugins.busy();
    let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);

    let unhandled = app
        .plugins
        .tree
        .input(SOURCE_PANE, Input::Key(Key::Char('j')), &cx);
    assert!(!unhandled.handled, "the tree ignores another pane's keys");

    let before = app.plugins.tree.cursor;
    let handled = app
        .plugins
        .tree
        .input(TREE_PANE, Input::Key(Key::Char('j')), &cx);
    assert!(handled.handled);
    assert!(app.plugins.tree.cursor > before);

    let unbound = app
        .plugins
        .tree
        .input(TREE_PANE, Input::Key(Key::Char('x')), &cx);
    assert!(!unbound.handled, "an unbound key is unhandled");
}

#[test]
fn files_changed_selects_the_nearest_visible_file() {
    let mut app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("b.rs", "b\n"),
        source_outline("c.rs", "c\n"),
    ]);
    app.plugins.tree.cursor = app
        .plugins
        .tree
        .row_of_file(&RepoPath::new("c.rs").unwrap())
        .unwrap();

    // Drop the selection and shrink the set to `[a, b]`. The tree cursor stays
    // past the end, so the fallback picks the nearest visible file at or after
    // the hint, then the last.
    app.workspace.selected = None;
    app.workspace.files = tui_core::workspace::FileSet {
        paths: std::sync::Arc::from(vec![
            RepoPath::new("a.rs").unwrap(),
            RepoPath::new("b.rs").unwrap(),
        ]),
        ..Default::default()
    };

    let specs = tui_core::app::Specs::panes_only(&app);
    let overlay = app.kernel.overlay;
    let busy = app.plugins.busy();
    let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);
    let outs = app.plugins.tree.on_event(&Event::FilesChanged, &cx);

    assert!(
        matches!(
            outs.as_slice(),
            [Out::Kernel(Request::Select(Some(path)))]
                if path == &RepoPath::new("b.rs").unwrap()
        ),
        "the fallback selects the nearest visible file"
    );
}

#[test]
fn a_selection_change_reveals_the_file_in_the_tree() {
    let files: Vec<SourceOutline> = (0..40)
        .map(|index| source_outline(&format!("f{index:02}.rs"), "x\n"))
        .collect();
    let mut app = show_app(files);
    app.workspace.selected = Some(RepoPath::new("f39.rs").unwrap());

    let specs = tui_core::app::Specs::panes_only(&app);
    let overlay = app.kernel.overlay;
    let busy = app.plugins.busy();
    let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);
    app.plugins.tree.on_event(&Event::SelectionChanged, &cx);

    assert_eq!(app.plugins.tree.cursor, 39);
}

#[cfg(feature = "diff")]
#[test]
fn the_tree_runs_its_resize_commands_and_the_views_do_not() {
    let mut app = two_files();
    let specs = tui_core::app::Specs::panes_only(&app);
    let overlay = app.kernel.overlay;
    let busy = app.plugins.busy();
    let cx = tui_core::app::cx_for(&app.kernel, overlay, busy, &specs);

    let tree_command = |name: &'static str| CommandId { owner: TREE, name };
    let wider = app.plugins.tree.run(tree_command("wider"), &cx);
    assert!(matches!(
        wider.as_slice(),
        [Out::Kernel(Request::ResizeNavigator(
            NavigatorResize::Wider
        ))]
    ));
    let narrower = app.plugins.tree.run(tree_command("narrower"), &cx);
    assert!(matches!(
        narrower.as_slice(),
        [Out::Kernel(Request::ResizeNavigator(
            NavigatorResize::Narrower
        ))]
    ));
    let reset = app.plugins.tree.run(tree_command("reset"), &cx);
    assert!(matches!(
        reset.as_slice(),
        [Out::Kernel(Request::ResizeNavigator(
            NavigatorResize::Reset
        ))]
    ));
    assert!(
        app.plugins
            .tree
            .run(tree_command("unknown"), &cx)
            .is_empty()
    );

    // The views must not also map a tree command, or the resize runs twice.
    let guarded = app.plugins.source.run(tree_command("wider"), &cx);
    assert!(
        guarded.is_empty(),
        "the source view must not double-run a tree command"
    );
    let guarded = app.plugins.diff.run(tree_command("wider"), &cx);
    assert!(
        guarded.is_empty(),
        "the diff view must not double-run a tree command"
    );

    let mut cmds = Vec::new();
    apply_request(
        &mut app,
        Request::RunCommand(tree_command("wider")),
        &mut cmds,
    );
    assert_eq!(app.chrome.tree_percent, TREE_DEFAULT_PERCENT + TREE_STEP);
}

#[test]
fn dropping_the_selection_does_not_reset_the_body() {
    let mut app = folder_app();
    app.plugins.source.body.scroll = 2;
    app.plugins.source.body.hscroll = 1;

    let set = tui_core::workspace::FileSet {
        paths: std::sync::Arc::from(Vec::new()),
        ..Default::default()
    };
    let mut cmds = Vec::new();
    apply_request(&mut app, Request::SetFiles(set), &mut cmds);

    assert!(app.kernel.workspace.selected.is_none());
    assert_eq!(
        app.plugins.source.body.scroll, 2,
        "dropping a selection must not reset the body"
    );
    assert_eq!(app.plugins.source.body.hscroll, 1);
    assert!(cmds.is_empty());
}

/// The migrated kernel-level validation over the real registry: every view and
/// every focus stop must produce a conflict-free command table.
#[cfg(feature = "diff")]
#[test]
fn the_real_registry_validates_in_every_view_and_focus() {
    let show = folder_app();
    let range = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let commits = commits_app(3);

    for app in [&show, &range, &commits] {
        let specs = tui_core::app::collect_specs(app).commands;
        for focus in [
            Some(TREE_FOCUS),
            Some(SOURCE_FOCUS),
            Some(DIFF_BODY_FOCUS),
            Some(DIFF_BAND_FOCUS),
            None,
        ] {
            assert!(
                tui_core::command::validate(&specs, focus).is_ok(),
                "focus {focus:?}: {:?}",
                tui_core::command::validate(&specs, focus)
            );
        }
    }

    let folds = |app: &App| -> usize {
        tui_core::app::collect_specs(app)
            .commands
            .iter()
            .filter(|spec| spec.id.owner == SOURCE && spec.id.name.starts_with("fold."))
            .count()
    };
    assert_eq!(folds(&show), 7);
    assert_eq!(folds(&range), 0);
    assert_eq!(folds(&commits), 0);
}
