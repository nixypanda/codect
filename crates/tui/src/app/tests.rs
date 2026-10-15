use super::*;
use crate::component::overlay::Overlay;
use crate::component::text_input::TextInput;
use crate::page::show::ShowHighlights;
use crate::page::{DiffSide, DiffViewState, Loaded, Search, SearchMatch, SearchSide, ShowFocus};
use crate::render::icons::IconStyle;
use crate::render::theme::Theme;
use crate::view::view;
use base::{
    Area, AreaSet, ItemKind, ProjectedItem, RepoPath, Selection, SelectionGroup, SourceSpan,
    SupportedPath,
};
use engine::CommitStep;
use ratatui::style::Color;

fn item(text: &str) -> ProjectedItem {
    ProjectedItem {
        stable_key: "item".to_owned(),
        parent_key: None,
        kind: ItemKind::Function,
        name: "item".to_owned(),
        span: SourceSpan::new(0, 0, 0, 0, 0, 0),
        canonical_text: text.to_owned(),
    }
}

fn projected(path: &str, text: &str) -> ProjectedFile {
    let path = SupportedPath::new(RepoPath::new(path).expect("valid path"))
        .expect("test path is supported");
    if text.is_empty() {
        return ProjectedFile::try_new(path, Vec::new()).expect("valid fixture");
    }
    ProjectedFile::try_new(path, vec![item(text)]).expect("valid fixture")
}

fn show_request() -> ShowRequest {
    ShowRequest {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
    }
}

fn diff_request() -> DiffRequest {
    DiffRequest {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Range,
        merge_base: false,
    }
}

fn commits_request() -> DiffRequest {
    DiffRequest {
        view: DiffView::Commits,
        ..diff_request()
    }
}

fn file_diff(path: &str, old: Option<&str>, new: Option<&str>) -> FileDiff {
    match (old, new) {
        (None, Some(text)) => FileDiff::Added {
            new: projected(path, text),
        },
        (Some(text), None) => FileDiff::Deleted {
            old: projected(path, text),
        },
        (Some(old_text), Some(new_text)) => FileDiff::Modified {
            old: projected(path, old_text),
            new: projected(path, new_text),
        },
        (None, None) => panic!("a test diff needs at least one side"),
    }
}

fn base_app(request: LoadRequest) -> App {
    App::new(
        "/repo".to_owned(),
        request,
        "all".to_owned(),
        100,
        30,
        Theme::dark(),
        Icons::new(IconStyle::None),
    )
}

fn show_app(files: Vec<ProjectedFile>) -> App {
    let app = base_app(LoadRequest::Show {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
    });
    let (next, _) = update(
        Msg::ShowLoaded {
            request: show_request(),
            result: Ok(files.into()),
        },
        &app,
    );
    settle(next)
}

fn diff_app(diffs: Vec<FileDiff>) -> App {
    let app = base_app(LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Range,
        merge_base: false,
    });
    let (next, _) = update(
        Msg::DiffLoaded {
            request: diff_request(),
            result: Ok(diffs.into()),
        },
        &app,
    );
    settle(next)
}

fn commit_steps(count: usize) -> Vec<CommitStep> {
    use git::{HashKind, ObjectId};
    (0..count)
        .map(|index| CommitStep {
            parent_id: ObjectId {
                kind: HashKind::Sha1,
                bytes: vec![index as u8; 20],
            },
            commit_id: ObjectId {
                kind: HashKind::Sha1,
                bytes: vec![(index + 1) as u8; 20],
            },
            subject: format!("Commit subject {index}"),
        })
        .collect()
}

fn commits_app(count: usize) -> App {
    let app = base_app(LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Commits,
        merge_base: false,
    });
    let (next, _) = update(
        Msg::HistoryLoaded {
            request: commits_request(),
            result: Ok((
                commit_steps(count).into(),
                vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into(),
            )),
        },
        &app,
    );
    settle(next)
}

fn show_ref(app: &App) -> &Show {
    match &app.loaded {
        Loaded::Show(show) => show,
        _ => panic!("expected a show"),
    }
}

fn diff_ref(app: &App) -> &Diff {
    match &app.loaded {
        Loaded::Diff(diff) => diff,
        _ => panic!("expected a diff"),
    }
}

fn commits_ref(app: &App) -> &crate::component::commit_picker::CommitPicker {
    match &app.loaded {
        Loaded::Diff(diff) => match &diff.view {
            DiffViewState::Commits(commits) => &commits.picker,
            _ => panic!("expected commits"),
        },
        _ => panic!("expected a diff"),
    }
}

fn selected(app: &App) -> Option<String> {
    app.loaded.selected().map(ToString::to_string)
}

fn diff_focus_of(app: &App) -> Focus {
    app.loaded.focus()
}

fn focus_content(app: &mut App) {
    app.loaded.set_focus(Focus::Content);
}

fn two_files() -> App {
    show_app(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")])
}

// -----------------------------------------------------------------------
// Install and selection
// -----------------------------------------------------------------------

#[test]
fn a_loaded_result_installs_files_and_preserves_the_selection() {
    let app = two_files();
    let (next, _) = update(
        Msg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![projected("a.rs", "A\n"), projected("b.rs", "B\n")].into()),
        },
        &app,
    );

    assert_eq!(next.loaded.mode(), ProjectionMode::Signatures);
    assert_eq!(selected(&next).as_deref(), Some("a.rs"));
    assert_eq!(show_ref(&next).active_text(), Some("A\n"));
}

#[test]
fn a_reload_that_drops_the_selected_file_picks_the_nearest_visible_one() {
    let mut app = show_app(vec![
        projected("a.rs", "a\n"),
        projected("b.rs", "b\n"),
        projected("c.rs", "c\n"),
    ]);
    app.tree.cursor = app
        .tree
        .row_of_file(&RepoPath::new("b.rs").unwrap())
        .unwrap();
    app.loaded
        .set_selected(Some(RepoPath::new("b.rs").unwrap()));

    let (next, _) = update(
        Msg::ShowLoaded {
            request: show_request(),
            result: Ok(vec![projected("a.rs", "a\n"), projected("c.rs", "c\n")].into()),
        },
        &app,
    );
    assert_eq!(selected(&next).as_deref(), Some("c.rs"));
}

#[test]
fn a_failed_reload_keeps_the_last_model_and_shows_a_diagnostic() {
    let app = two_files();
    let error = EngineError::Selection(base::SelectionError::UnknownArea {
        name: "x".to_owned(),
    });
    let (next, _) = update(
        Msg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Err(Box::new(error)),
        },
        &app,
    );

    assert_eq!(
        next.loaded.mode(),
        ProjectionMode::Types,
        "mode must not change"
    );
    assert_eq!(show_ref(&next).files.len(), 2);
    assert!(next.diagnostic.is_some());
    assert!(next.loaded.selected().is_some());
}

#[test]
fn empty_projections_are_hidden_from_the_tree() {
    let app = show_app(vec![projected("empty.rs", ""), projected("full.rs", "x\n")]);
    let labels: Vec<&str> = app.tree.rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, vec!["full.rs"]);
    assert!(
        !app.loaded
            .visible()
            .contains(&RepoPath::new("empty.rs").unwrap())
    );
}

#[test]
fn a_diff_tree_contains_only_changed_paths() {
    let app = diff_app(vec![
        file_diff("src/a.rs", Some("a\n"), Some("A\n")),
        file_diff("src/b.rs", None, Some("b\n")),
    ]);
    let labels: Vec<&str> = app.tree.rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, vec!["src", "a.rs", "b.rs"]);
}

#[test]
fn tree_rows_follow_raw_path_order_and_are_nested() {
    let app = show_app(vec![
        projected("a.rs", "a\n"),
        projected("src/lib.rs", "l\n"),
        projected("src/main.rs", "m\n"),
    ]);
    let labels: Vec<&str> = app.tree.rows.iter().map(|row| row.label.as_str()).collect();
    assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
    assert_eq!(app.tree.rows[0].depth, 0);
    assert_eq!(app.tree.rows[1].depth, 0);
    assert_eq!(app.tree.rows[2].depth, 1);
}

#[test]
fn collapsing_a_directory_hides_its_children_and_expanding_restores_them() {
    let mut app = show_app(vec![
        projected("a.rs", "a\n"),
        projected("src/lib.rs", "l\n"),
        projected("src/main.rs", "m\n"),
    ]);
    app.tree.cursor = 1;
    assert!(matches!(
        app.tree.current_row().unwrap().kind,
        RowKind::Directory { .. }
    ));

    let (collapsed, _) = update(Msg::Key(Key::Enter), &app);
    let labels: Vec<&str> = collapsed
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["a.rs", "src"]);
    assert!(matches!(
        collapsed.tree.rows[1].kind,
        RowKind::Directory {
            expanded: false,
            ..
        }
    ));

    let (expanded, _) = update(Msg::Key(Key::Right), &collapsed);
    let labels: Vec<&str> = expanded
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
        projected("a.rs", "a\n"),
        projected("src/lib.rs", "l\n"),
    ]);
    app.tree.cursor = 1;
    let (folded, _) = update(Msg::Key(Key::Left), &app);
    assert!(matches!(
        folded.tree.rows[1].kind,
        RowKind::Directory {
            expanded: false,
            ..
        }
    ));
}

// -----------------------------------------------------------------------
// Commits
// -----------------------------------------------------------------------

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
        Msg::HistoryLoaded {
            request: commits_request(),
            result: Ok((Vec::new().into(), Vec::new().into())),
        },
        &app,
    );
    assert!(commands.is_empty());
    assert_eq!(next.loaded.mode(), ProjectionMode::Types);
    assert!(commits_ref(&next).is_empty());
    assert!(next.loaded.visible().is_empty());
    assert!(next.loaded.selected().is_none());
    let (unchanged, commands) = update(Msg::Key(Key::Down), &next);
    assert!(commands.is_empty());
    assert_eq!(commits_ref(&unchanged).cursor(), 0);
}

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

#[test]
fn commit_picker_mouse_uses_drawn_geometry_and_loads_selected_step() {
    let app = commits_app(12);
    let (mut clicked, mut commands) = update(Msg::Mouse(click(4, 3)), &app);
    coalesce_commit_loads(&mut clicked, &mut commands);
    assert_eq!(diff_focus_of(&clicked), Focus::Commits);
    assert!(matches!(commands.as_slice(), [Cmd::Step { index: 1, .. }]));

    let (mut wheeled, mut commands) = update(Msg::Mouse(scroll(4, 3, false)), &app);
    coalesce_commit_loads(&mut wheeled, &mut commands);
    assert_eq!(diff_focus_of(&wheeled), Focus::Commits);
    assert!(matches!(commands.as_slice(), [Cmd::Step { index: 3, .. }]));

    let mut narrow = app.clone();
    narrow.size = Size {
        width: 60,
        height: 30,
    };
    let (mut narrow_clicked, mut commands) = update(Msg::Mouse(click(4, 3)), &narrow);
    coalesce_commit_loads(&mut narrow_clicked, &mut commands);
    assert!(matches!(commands.as_slice(), [Cmd::Step { index: 1, .. }]));

    let (files, commands) = update(Msg::Mouse(click(4, 11)), &app);
    assert_eq!(diff_focus_of(&files), Focus::Tree);
    assert!(commands.is_empty());
}

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
    assert!(matches!(commands.as_slice(), [Cmd::Step { index: 3, .. }]));
    assert_eq!(diff_ref(&app).diffs(), displayed.as_slice());
    assert_eq!(diff_ref(&app).revisions(), displayed_labels);

    let (loaded, _) = update(
        Msg::StepLoaded {
            index: 3,
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("new\n"))].into()),
        },
        &app,
    );
    assert_eq!(commits_ref(&loaded).cursor(), 3);
    assert_eq!(commits_ref(&loaded).target(), None);
    assert_ne!(diff_ref(&loaded).revisions(), displayed_labels);
    assert_eq!(loaded.loaded.selected(), app.loaded.selected());

    let (failed, commands) = update(
        Msg::StepLoaded {
            index: 3,
            result: Err(Box::new(EngineError::Selection(
                base::SelectionError::UnknownArea {
                    name: "missing".to_owned(),
                },
            ))),
        },
        &app,
    );
    assert!(commands.is_empty());
    assert_eq!(commits_ref(&failed).cursor(), 0);
    assert_eq!(commits_ref(&failed).target(), None);
    assert_eq!(diff_ref(&failed).diffs(), displayed.as_slice());
    assert_eq!(diff_ref(&failed).revisions(), displayed_labels);
    assert!(failed.diagnostic.is_some());
}

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

#[test]
fn palette_switches_between_range_and_commit_views() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    assert!(
        app.loaded
            .palette_entries()
            .iter()
            .any(|(action, ..)| *action == Action::Range(RangeAction::SwitchToCommits))
    );
    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_action(
        &mut next,
        Action::Range(RangeAction::SwitchToCommits),
        &mut commands,
    );
    assert_eq!(
        commands,
        vec![Cmd::Diff(DiffRequest {
            view: DiffView::Commits,
            ..diff_request()
        })]
    );
    assert_eq!(diff_ref(&next).revisions(), diff_ref(&app).revisions());
}

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
        Msg::DiffLoaded {
            request,
            result: Ok(vec![file_diff("a.rs", None, Some("a\n"))].into()),
        },
        &app,
    );
    let app = settle(next);

    // The commits view needs a first-parent chain, so it is neither offered nor
    // performed for a snapshot range.
    assert!(
        !app.loaded
            .palette_entries()
            .iter()
            .any(|(action, ..)| *action == Action::Range(RangeAction::SwitchToCommits))
    );

    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_action(
        &mut next,
        Action::Range(RangeAction::SwitchToCommits),
        &mut commands,
    );
    assert!(commands.is_empty(), "no first-parent load may be issued");
    assert!(next.diagnostic.is_some(), "the refusal is explained");
}

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
        Msg::DiffLoaded {
            request,
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into()),
        },
        &app,
    );
    let app = settle(next);

    // The mode survives the load and a request built from the page.
    assert!(diff_ref(&app).merge_base);
    assert!(diff_ref(&app).request().merge_base);

    // As with a snapshot range, the commits view is not offered.
    assert!(
        !app.loaded
            .palette_entries()
            .iter()
            .any(|(action, ..)| *action == Action::Range(RangeAction::SwitchToCommits))
    );
    let mut next = app.clone();
    let mut commands = Vec::new();
    apply_action(
        &mut next,
        Action::Range(RangeAction::SwitchToCommits),
        &mut commands,
    );
    assert!(commands.is_empty(), "no first-parent load may be issued");
    assert!(next.diagnostic.is_some(), "the refusal is explained");
}

// -----------------------------------------------------------------------
// Settle deferral
// -----------------------------------------------------------------------

#[test]
fn navigation_defers_highlighting_until_settle() {
    let app = show_app(vec![
        projected("a.rs", "pub fn a();\n"),
        projected("b.rs", "pub fn b();\n"),
    ]);
    // Force a fresh selection that has not been highlighted.
    let mut fresh = app.clone();
    if let Loaded::Show(show) = &mut fresh.loaded {
        show.highlight = ShowHighlights::default();
    }
    assert!(show_ref(&fresh).active_lines().is_none());

    let (moved, _) = update(Msg::Key(Key::Down), &fresh);
    assert!(
        show_ref(&moved).active_lines().is_none(),
        "update must not highlight"
    );

    let settled = settle(moved);
    assert!(show_ref(&settled).active_lines().is_some());
}

#[test]
fn settle_skips_highlighting_when_the_body_is_hidden() {
    let mut app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
    app.size = Size {
        width: 60,
        height: 30,
    };
    app.tree.cursor = 0;
    if let Loaded::Show(show) = &mut app.loaded {
        show.focus = ShowFocus::Tree;
        show.highlight = ShowHighlights::default();
    }
    let settled = settle(app);
    assert!(show_ref(&settled).active_lines().is_none());
}

#[test]
fn the_highlight_cache_is_bounded() {
    let mut cache: crate::util::cache::BoundedCache<usize> =
        crate::util::cache::BoundedCache::default();
    for index in 0..(crate::util::cache::CAP + 10) {
        let path = RepoPath::new(format!("f{index}.rs")).expect("valid path");
        cache.insert(path, index);
    }
    assert!(!cache.contains_key(&RepoPath::new("f0.rs").expect("valid path")));
    let newest = RepoPath::new(format!("f{}.rs", crate::util::cache::CAP + 9)).expect("valid path");
    assert!(cache.contains_key(&newest));
}

// -----------------------------------------------------------------------
// Mode and overlays
// -----------------------------------------------------------------------

#[test]
fn mode_picker_defers_the_mode_change_until_selection() {
    let app = two_files();
    let (opened, cmds) = update(Msg::Key(Key::Char('m')), &app);
    assert!(cmds.is_empty());
    assert!(matches!(opened.overlay, Some(Overlay::Mode { .. })));

    let (down, _) = update(Msg::Key(Key::Down), &opened);
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);

    assert_eq!(app.loaded.mode(), ProjectionMode::Types);
    assert_eq!(
        chosen.loaded.mode(),
        ProjectionMode::Types,
        "mode changes on load"
    );
    assert_eq!(chosen.overlay, None);
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            mode: ProjectionMode::Signatures,
            ..show_request()
        })]
    );
}

#[test]
fn mode_picker_on_a_diff_keeps_the_diff_shape() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let (opened, _) = update(Msg::Key(Key::Char('m')), &app);
    let (down, _) = update(Msg::Key(Key::Down), &opened);
    let (_, cmds) = update(Msg::Key(Key::Enter), &down);
    assert_eq!(
        cmds,
        vec![Cmd::Diff(DiffRequest {
            mode: ProjectionMode::Signatures,
            ..diff_request()
        })]
    );
}

#[test]
fn the_mode_picker_lists_the_modes() {
    let mut app = two_files();
    app.overlay = Some(Overlay::mode(ProjectionMode::Types));
    let text = buffer_text(&render(&app, 100, 20));
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
    // The mode applies on load, so the selection is observed as a command.
    let (_, cmds) = update(Msg::Key(Key::Enter), &down_again);
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            mode: ProjectionMode::Tests,
            ..show_request()
        })]
    );

    // A further Down must stay on the last mode rather than wrap or panic.
    let (clamped, _) = update(Msg::Key(Key::Down), &down_again);
    let (_, cmds) = update(Msg::Key(Key::Enter), &clamped);
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            mode: ProjectionMode::Tests,
            ..show_request()
        })]
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
    let mut app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
    app.chrome.tree_percent = TREE_MAX_PERCENT;
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Files"), "{text}");
    assert!(text.contains("pub fn a();"), "{text}");
}

#[test]
fn tab_toggles_the_tree_and_both_diff_panes() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let (step, _) = update(Msg::Key(Key::Tab), &app);
    assert!(diff_ref(&step).focus_is_diff());
    let (step, _) = update(Msg::Key(Key::Tab), &step);
    assert_eq!(diff_focus_of(&step), Focus::Tree);
    let (step, _) = update(Msg::Key(Key::BackTab), &step);
    assert!(diff_ref(&step).focus_is_diff());

    let app = two_files();
    let (step, _) = update(Msg::Key(Key::Tab), &app);
    assert_eq!(show_ref(&step).focus, ShowFocus::Body);
    let (step, _) = update(Msg::Key(Key::BackTab), &step);
    assert_eq!(show_ref(&step).focus, ShowFocus::Tree);
}

#[test]
fn help_toggles_and_swallows_navigation() {
    let app = two_files();
    let (help, _) = update(Msg::Key(Key::Char('?')), &app);
    assert_eq!(help.overlay, Some(Overlay::Help));

    let (still, _) = update(Msg::Key(Key::Char('j')), &help);
    assert_eq!(still.tree.cursor, help.tree.cursor);
    assert_eq!(still.overlay, Some(Overlay::Help));

    let (closed, _) = update(Msg::Key(Key::Esc), &help);
    assert_eq!(closed.overlay, None);
}

#[test]
fn revision_prompt_prefills_the_current_revision() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    let prompt = show_ref(&opened).prompt.as_ref().expect("prompt");
    assert_eq!(prompt.text, "HEAD");
    assert_eq!(prompt.cursor, 4);
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
    assert!(show_ref(&applied).prompt.is_none());
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            revision: "HEAD~2".to_owned(),
            ..show_request()
        })]
    );
}

#[test]
fn escaping_a_revision_prompt_emits_nothing() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('r')), &app);
    let (closed, cmds) = update(Msg::Key(Key::Esc), &opened);
    assert!(show_ref(&closed).prompt.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn diff_revision_keys_target_base_and_target_fields() {
    let app = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("b\n"))]);
    let (base, _) = update(Msg::Key(Key::Char('b')), &app);
    assert_eq!(
        diff_ref(&base).prompt.as_ref().map(|p| p.side),
        Some(DiffSide::Base)
    );
    let (target, _) = update(Msg::Key(Key::Char('t')), &app);
    assert_eq!(
        diff_ref(&target).prompt.as_ref().map(|p| p.side),
        Some(DiffSide::Target)
    );
}

#[test]
fn the_revision_prompt_renders_its_label_and_text() {
    let mut app = two_files();
    if let Loaded::Show(show) = &mut app.loaded {
        show.prompt = Some(TextInput::new("HEAD~2"));
    }
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("revision"), "{text}");
    assert!(text.contains("HEAD~2"), "{text}");
}

fn area_set() -> AreaSet {
    AreaSet::new([
        Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
        Area::new("web", [RepoPath::new("src/web").unwrap()]).unwrap(),
    ])
    .unwrap()
}

#[test]
fn scope_key_opens_the_chooser_and_requests_areas() {
    let app = two_files();
    let (opened, cmds) = update(Msg::Key(Key::Char('s')), &app);
    assert!(matches!(opened.overlay, Some(Overlay::Scope(_))));
    assert_eq!(cmds, vec![Cmd::LoadAreas]);
}

#[test]
fn choosing_all_from_the_chooser_loads_everything() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &loaded);
    assert_eq!(chosen.overlay, None);
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            selection: Selection::all(),
            ..show_request()
        })]
    );
}

#[test]
fn selecting_an_area_loads_that_area() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
    let (down, _) = update(Msg::Key(Key::Down), &loaded);
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);
    assert_eq!(chosen.overlay, None);
    let expected = Selection::new(vec![SelectionGroup::Area(
        Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
    )]);
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            selection: expected,
            ..show_request()
        })]
    );
}

#[test]
fn a_config_error_keeps_the_chooser_open_with_a_message() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let error = EngineError::Selection(base::SelectionError::UnknownArea {
        name: "x".to_owned(),
    });
    let (loaded, _) = update(Msg::AreasLoaded(Err(Box::new(error))), &opened);
    match loaded.overlay {
        Some(Overlay::Scope(chooser)) => assert!(chooser.error.is_some()),
        other => panic!("expected a scope chooser, got {other:?}"),
    }
}

#[test]
fn literal_path_entry_builds_a_path_selection() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('s')), &app);
    let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
    let mut state = loaded;
    for _ in 0..3 {
        let (next, _) = update(Msg::Key(Key::Down), &state);
        state = next;
    }
    let (input, _) = update(Msg::Key(Key::Enter), &state);
    let mut typed = input;
    for character in "src/lib.rs".chars() {
        let (next, _) = update(Msg::Key(Key::Char(character)), &typed);
        typed = next;
    }
    let (chosen, cmds) = update(Msg::Key(Key::Enter), &typed);
    assert_eq!(chosen.overlay, None);
    let expected = Selection::new(vec![SelectionGroup::Path(
        RepoPath::new("src/lib.rs").unwrap(),
    )]);
    assert_eq!(
        cmds,
        vec![Cmd::Show(ShowRequest {
            selection: expected,
            ..show_request()
        })]
    );
}

#[test]
fn search_finds_matches_and_steps_them() {
    let app = show_app(vec![projected("a.rs", "alpha\nbeta\nalpha again\n")]);
    let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
    assert!(matches!(opened.overlay, Some(Overlay::Search(_))));

    let mut state = opened;
    for character in "alpha".chars() {
        let (next, _) = update(Msg::Key(Key::Char(character)), &state);
        state = next;
    }
    assert_eq!(state.loaded.search().expect("live search").matches.len(), 2);

    let (committed, _) = update(Msg::Key(Key::Enter), &state);
    assert_eq!(committed.overlay, None);
    let (stepped, _) = update(Msg::Key(Key::Char('n')), &committed);
    assert_eq!(stepped.loaded.search().unwrap().cursor, 1);
    let (back, _) = update(Msg::Key(Key::Char('N')), &stepped);
    assert_eq!(back.loaded.search().unwrap().cursor, 0);
}

#[test]
fn switching_files_recomputes_a_committed_search() {
    let mut app = show_app(vec![
        projected("a.rs", "alpha\n"),
        projected("b.rs", "beta\nalpha\n"),
    ]);
    if let Loaded::Show(show) = &mut app.loaded {
        show.search = Some(Search {
            needle: "alpha".to_owned(),
            matches: vec![SearchMatch {
                side: SearchSide::Show,
                line: 1,
                start: 0,
                end: 5,
            }],
            cursor: 0,
        });
    }
    app.loaded
        .set_selected(Some(RepoPath::new("b.rs").unwrap()));
    if let Loaded::Show(show) = &mut app.loaded {
        show.resync_search();
    }
    let search = app.loaded.search().expect("search");
    assert_eq!(search.matches.len(), 1);
    assert_eq!(search.matches[0].line, 2, "the match moved to the new file");
}

#[test]
fn q_types_into_a_search_input_instead_of_quitting() {
    let app = two_files();
    let (opened, _) = update(Msg::Key(Key::Char('/')), &app);
    let (typed, _) = update(Msg::Key(Key::Char('q')), &opened);
    assert!(!typed.chrome.quit);
    match &typed.overlay {
        Some(Overlay::Search(state)) => assert_eq!(state.input.text, "q"),
        other => panic!("expected a search overlay, got {other:?}"),
    }
}

#[test]
fn the_palette_and_finder_render_their_lists() {
    let mut app = two_files();
    app.overlay = Some(Overlay::palette(&overlay::Ctx {
        visible: app.loaded.visible(),
        entries: &app.loaded.palette_entries(),
        mode: app.loaded.mode(),
    }));
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("commands"), "{text}");
    assert!(text.contains("Switch Types"), "{text}");

    let mut app = two_files();
    app.overlay = Some(Overlay::finder(&overlay::Ctx {
        visible: app.loaded.visible(),
        entries: &app.loaded.palette_entries(),
        mode: app.loaded.mode(),
    }));
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("find file"), "{text}");
    assert!(text.contains("a.rs"), "{text}");
}

// -----------------------------------------------------------------------
// Scrolling and mouse
// -----------------------------------------------------------------------

fn click(column: u16, row: u16) -> Mouse {
    Mouse {
        column,
        row,
        kind: MouseKind::Click,
    }
}

fn scroll(column: u16, row: u16, up: bool) -> Mouse {
    Mouse {
        column,
        row,
        kind: if up {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        },
    }
}

const TREE_X: u16 = 2;
const TREE_Y: u16 = 2;
const CONTENT_X: u16 = 50;

#[test]
fn page_keys_move_the_cursor_in_the_tree_and_scroll_the_body() {
    let app = show_app(vec![
        projected("a.rs", "one\ntwo\nthree\nfour\nfive\n"),
        projected("b.rs", "x\n"),
    ]);
    let (down, _) = update(Msg::Key(Key::PageDown), &app);
    assert!(down.tree.cursor > app.tree.cursor);

    let mut body = app.clone();
    focus_content(&mut body);
    let (scrolled, _) = update(Msg::Key(Key::PageDown), &body);
    assert!(show_ref(&scrolled).body.scroll > 0);
}

#[test]
fn body_scrolling_is_clamped_to_the_content() {
    let mut app = show_app(vec![projected("a.rs", "one\ntwo\nthree\n")]);
    focus_content(&mut app);
    for _ in 0..10 {
        let (next, _) = update(Msg::Key(Key::Char('j')), &app);
        app = next;
    }
    assert_eq!(show_ref(&app).body.scroll, 2);
    for _ in 0..10 {
        let (next, _) = update(Msg::Key(Key::Char('k')), &app);
        app = next;
    }
    assert_eq!(show_ref(&app).body.scroll, 0);
}

#[test]
fn clicking_a_file_selects_it() {
    let app = two_files();
    let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
    assert_eq!(selected(&next).as_deref(), Some("b.rs"));
    assert_eq!(next.tree.cursor, 1);
    assert!(matches!(next.loaded, Loaded::Show(ref show) if show.focus == ShowFocus::Tree));
}

#[test]
fn clicking_a_directory_folds_it() {
    let app = show_app(vec![
        projected("a.rs", "a\n"),
        projected("src/lib.rs", "l\n"),
    ]);
    let (folded, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
    let labels: Vec<&str> = folded
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(labels, vec!["a.rs", "src"]);
}

#[test]
fn the_wheel_moves_the_tree_cursor_and_selects_as_it_passes() {
    let app = show_app(vec![
        projected("a.rs", "a\n"),
        projected("b.rs", "b\n"),
        projected("c.rs", "c\n"),
    ]);
    let (moved, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &app);
    assert_eq!(
        moved.tree.cursor, 2,
        "the wheel step clamps to the last row"
    );
    assert_eq!(selected(&moved).as_deref(), Some("c.rs"));
}

#[test]
fn the_wheel_scrolls_the_content_and_focuses_it() {
    let app = show_app(vec![projected("a.rs", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")]);
    let (down, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &app);
    assert!(matches!(down.loaded, Loaded::Show(ref show) if show.focus == ShowFocus::Body));
    assert_eq!(show_ref(&down).body.scroll, 3);
    let (up, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, true)), &down);
    assert_eq!(show_ref(&up).body.scroll, 0);
}

#[test]
fn the_wheel_scrolls_a_diff_and_clicking_it_focuses() {
    let app = diff_app(vec![file_diff(
        "a.rs",
        Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"),
        Some("1\n2\n3\n4\n5\n6\n7\n8\n9\nX\n"),
    )]);
    let (scrolled, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &app);
    assert!(diff_ref(&scrolled).focus_is_diff());
    assert!(diff_ref(&scrolled).body_scroll() > 0);

    let (clicked, _) = update(Msg::Mouse(click(CONTENT_X, 5)), &app);
    assert!(diff_ref(&clicked).focus_is_diff());
    assert_eq!(diff_ref(&clicked).body_scroll(), 0);
}

#[test]
fn mouse_is_ignored_while_an_overlay_is_open() {
    let mut app = two_files();
    app.overlay = Some(Overlay::Help);
    let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &app);
    assert_eq!(next.tree.cursor, app.tree.cursor);
    assert_eq!(next.loaded.selected(), app.loaded.selected());
}

#[test]
fn a_click_on_the_header_or_a_divider_does_nothing() {
    let app = two_files();
    let (header, _) = update(Msg::Mouse(click(5, 0)), &app);
    assert_eq!(header.tree.cursor, app.tree.cursor);

    let mut body_focused = app.clone();
    focus_content(&mut body_focused);
    let (divider, _) = update(Msg::Mouse(click(29, 5)), &body_focused);
    assert!(matches!(divider.loaded, Loaded::Show(ref show) if show.focus == ShowFocus::Body));
    assert_eq!(divider.loaded.selected(), app.loaded.selected());
}

// -----------------------------------------------------------------------
// Status, diagnostic, spinner
// -----------------------------------------------------------------------

#[test]
fn the_status_bar_shows_mode_scope_and_hints() {
    let app = show_app(vec![projected("a.rs", "x\n")]);
    let buffer = render(&app, 120, 20);
    let header = buffer_row(&buffer, 0);
    let footer = buffer_row(&buffer, 19);
    assert!(header.contains("types"), "{header}");
    assert!(header.contains("HEAD"), "{header}");
    assert!(header.contains("scope:"), "{header}");
    assert!(!header.contains("a.rs"), "{header}");
    assert!(footer.contains("a.rs"), "{footer}");
    assert!(footer.contains("1 lines"), "{footer}");
    assert!(footer.contains("help"), "{footer}");
}

#[test]
fn narrow_chrome_keeps_context_and_the_selected_path() {
    let app = show_app(vec![projected("src/very/deep/file.rs", "x\n")]);
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
        Msg::ShowLoaded {
            request: ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            },
            result: Ok(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")].into()),
        },
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

// -----------------------------------------------------------------------
// Rendering
// -----------------------------------------------------------------------

fn render(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| view(app, frame)).expect("draw");
    terminal.backend().buffer().clone()
}

fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .filter_map(|x| buffer.cell((x, y)))
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn a_wide_terminal_shows_the_tree_and_the_projection() {
    let app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Files"), "tree pane missing: {text}");
    assert!(text.contains("pub fn a();"), "body missing: {text}");
}

#[test]
fn the_tree_keeps_a_quiet_selected_row_when_the_body_has_focus() {
    use crate::render::theme::{Capability, Flavor};

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
            projected("src/main.rs", "x\n"),
            projected("src/Main.elm", "y\n"),
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
    let app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
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
    let app = show_app(vec![projected("empty.rs", "")]);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("No files"), "{text}");
    assert!(
        text.contains("No projected file content in this scope."),
        "{text}"
    );
}

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
    let app = show_app(vec![projected("a.rs", &format!("{long}\n"))]);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("Files"), "the tree must survive: {text}");
    assert!(!text.contains(&long), "the long line must be clipped");
    assert!(text.contains(&"x".repeat(40)), "the visible prefix remains");
}

#[test]
fn responsive_boundaries_pick_the_right_layout() {
    let app = show_app(vec![projected("a.rs", "pub fn a();\n")]);
    let wide = buffer_text(&render(&app, 80, 20));
    assert!(wide.contains("Files") && wide.contains("pub fn a();"));

    let narrow = buffer_text(&render(&app, 79, 20));
    assert!(narrow.contains("Files") && !narrow.contains("pub fn a();"));

    assert!(!buffer_text(&render(&app, 40, 8)).contains("terminal too small"));
    assert!(buffer_text(&render(&app, 39, 8)).contains("terminal too small"));
    assert!(buffer_text(&render(&app, 100, 7)).contains("terminal too small"));
}

#[test]
fn horizontal_scroll_shifts_the_visible_columns() {
    let mut app = show_app(vec![projected("a.rs", "abcdef\n")]);
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
    if !crate::render::theme::colors_enabled() {
        return;
    }
    let app = show_app(vec![projected("a.rs", "pub struct User;\n")]);
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
    let file = ProjectedFile::try_new(path, vec![item("x\n")]).expect("valid fixture");
    let app = show_app(vec![file]);
    let labels: Vec<String> = app.tree.rows.iter().map(|row| row.label.clone()).collect();
    assert!(labels.iter().any(|label| label == "src"), "{labels:?}");
    assert!(
        labels.iter().any(|label| label.contains("\\xFF")),
        "{labels:?}"
    );
}

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
    app.overlay = Some(Overlay::Help);
    let text = buffer_text(&render(&app, 100, 20));
    assert!(text.contains("help"), "{text}");
    assert!(text.contains("quit"), "{text}");
}

#[test]
fn a_changed_diff_row_gets_delta_backgrounds() {
    if !crate::render::theme::colors_enabled() {
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
    use crate::render::theme::{Capability, Flavor};

    for flavor in [Flavor::Dark, Flavor::Light] {
        for overlay in [None, Some(Overlay::Help)] {
            let mut app = two_files();
            app.chrome.theme = Theme::new(flavor, Capability::TrueColor);
            app.overlay = overlay;
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
    use crate::render::theme::{Capability, Flavor};

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
    use crate::render::text::clip_line;
    assert_eq!(clip_line("abcdef", 2, 3), "cde");
    assert_eq!(clip_line("abcdef", 0, 0), "");
    assert_eq!(clip_line("abcdef", 10, 3), "");
    assert_eq!(clip_line("a\tb", 0, 10), "a    b");
    assert_eq!(clip_line("日本", 0, 3), "日");
    assert_eq!(clip_line("日本", 1, 4), "本");
}

#[test]
fn window_offset_keeps_the_cursor_visible() {
    use crate::render::layout::window_offset;
    assert_eq!(window_offset(0, 100, 10), 0);
    assert_eq!(window_offset(9, 100, 10), 0);
    assert_eq!(window_offset(10, 100, 10), 1);
    assert_eq!(window_offset(0, 3, 10), 0);
    assert_eq!(window_offset(5, 100, 0), 0);
}
