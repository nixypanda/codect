#![cfg(feature = "diff")]

//! Independent acceptance checks for moved files in the terminal frontend.
//!
//! These tests drive the same `update`/`settle`/`press`/`render` path as the
//! runtime and cover user-visible behavior the other app tests do not:
//! search navigation on both sides, one-empty-side moves, and history/range
//! reloads of a target-keyed move.

use base::{FileDiff, RepoPath, Selection, SelectionGroup};
use tui_core::app::{settle, update};
use tui_core::render::theme::Theme;
use tui_core::search::SearchPane;
use tui_core::util::input::Key;
use tui_diff::Msg as DiffMsg;

use crate::route::{DiffRequest, DiffView, LoadRequest};
use crate::test_support::{
    App, base_app, buffer_row, buffer_text, commit_steps, commits_app_with, commits_request,
    diff_app, diff_msg, diff_ref, diff_request, moved_file, press, render, selected,
};

fn scoped_diff_app(selection: Selection, diffs: Vec<FileDiff>) -> App {
    let request = DiffRequest {
        selection,
        ..diff_request()
    };
    let app = base_app(LoadRequest::Diff {
        base: request.base.clone(),
        target: request.target.clone(),
        mode: request.mode,
        selection: request.selection.clone(),
        view: DiffView::Range,
        merge_base: false,
    });
    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request,
            result: Ok(diffs.into()),
        }),
        &app,
    );
    settle(next)
}

fn search_of(app: &App) -> &tui_core::search::Search {
    app.workspace.search.as_ref().expect("a committed search")
}

fn tree_labels(app: &App) -> Vec<&str> {
    app.plugins
        .tree
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .collect()
}

fn file_row_count(app: &App) -> usize {
    app.plugins
        .tree
        .rows
        .iter()
        .filter(|row| matches!(row.kind, crate::tree::RowKind::File { .. }))
        .count()
}

/// Type `/needle` and press Enter through the real overlay/key dispatch path.
fn search_needle(app: &App) -> App {
    press(
        app,
        &[
            Key::Char('/'),
            Key::Char('n'),
            Key::Char('e'),
            Key::Char('e'),
            Key::Char('d'),
            Key::Char('l'),
            Key::Char('e'),
            Key::Enter,
        ],
    )
}

/// Count occurrences of `needle` whose first cell carries `background`.
fn highlighted_occurrences(
    buffer: &ratatui::buffer::Buffer,
    needle: &str,
    background: ratatui::style::Color,
) -> usize {
    let mut count = 0;
    for y in 0..buffer.area.height {
        let cells: Vec<&str> = (0..buffer.area.width)
            .filter_map(|x| buffer.cell((x, y)))
            .map(|cell| cell.symbol())
            .collect();
        let joined = cells.concat();
        let mut from = 0;
        while let Some(offset) = joined[from..].find(needle) {
            let byte = from + offset;
            let mut cell_offset = 0;
            for (index, symbol) in cells.iter().enumerate() {
                if cell_offset == byte {
                    if buffer
                        .cell((index as u16, y))
                        .is_some_and(|cell| cell.bg == background)
                    {
                        count += 1;
                    }
                    break;
                }
                cell_offset += symbol.len();
            }
            from = byte + needle.len();
        }
    }
    count
}

fn source_lines(needle_old: usize, needle_new: usize) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for line in 1..=100 {
        let old_line = if line == needle_old {
            "needle old".to_owned()
        } else if (10..=30).contains(&line) {
            format!("old body {line}")
        } else if (70..=90).contains(&line) {
            format!("old tail {line}")
        } else {
            format!("line {line}")
        };
        let new_line = if line == needle_new {
            "needle new".to_owned()
        } else if (10..=30).contains(&line) {
            format!("new body {line}")
        } else if (70..=90).contains(&line) {
            format!("new tail {line}")
        } else {
            format!("line {line}")
        };
        old.push_str(&old_line);
        old.push('\n');
        new.push_str(&new_line);
        new.push('\n');
    }
    (old, new)
}

#[test]
fn scope_selecting_the_old_path_renders_one_target_keyed_move_and_searches_it() {
    let old_path = RepoPath::new("src/old.rs").unwrap();
    let target = RepoPath::new("dst/new.rs").unwrap();
    let source = RepoPath::new("src/old.rs").unwrap();
    let selection = Selection::new(vec![SelectionGroup::Path(old_path)]);
    let app = scoped_diff_app(
        selection,
        vec![moved_file(
            "src/old.rs",
            "dst/new.rs",
            "needle one\n",
            "needle one\n",
        )],
    );

    assert_eq!(
        selected(&app).as_deref(),
        Some("dst/new.rs"),
        "the move is selected and keyed by its target even when only the old path is in scope"
    );
    assert_eq!(
        app.workspace.query.scope.label, "src/old.rs",
        "the header keeps the scope the user requested"
    );
    assert_eq!(
        app.workspace.files.paths.as_ref(),
        std::slice::from_ref(&target)
    );
    assert_eq!(file_row_count(&app), 1, "the move is one tree event");
    assert_eq!(diff_ref(&app).diffs().len(), 1);
    assert_eq!(
        app.workspace.files.moves.get(&target),
        Some(&source),
        "the tree label resolves the source from the target key"
    );

    assert_eq!(
        tree_labels(&app),
        vec!["dst", "src/old.rs → new.rs"],
        "the row lives under the target directory and names the source"
    );

    let text = buffer_text(&render(&app, 160, 30));
    assert!(text.contains("scope: src/old.rs"), "{text}");
    assert!(text.contains("BASE · src/old.rs"), "{text}");
    assert!(text.contains("TARGET · dst/new.rs"), "{text}");
    assert!(
        text.contains("src/old.rs → new.rs R"),
        "the target row must carry the move badge: {text}"
    );
    assert_eq!(
        text.matches("Moved from src/old.rs to dst/new.rs.").count(),
        2,
        "both panes state the move: {text}"
    );
    assert!(!text.contains("No text changes"), "{text}");
    assert!(!text.contains("@@"), "a pure move has no hunks: {text}");
    assert!(
        text.contains("dst/new.rs +0 −0"),
        "the single event has no line counts: {text}"
    );

    // Search through the actual overlay and key path. Even though the target
    // is outside the requested scope, selection uses the target key so both
    // projections are searched.
    let searched = search_needle(&app);
    let search = search_of(&searched);
    assert_eq!(search.matches.len(), 2, "both sides of the move match");
    assert_eq!(search.matches[0].pane, SearchPane::Old);
    assert_eq!(search.matches[1].pane, SearchPane::New);
    assert_eq!(search.cursor, 0);
    let searched_text = buffer_text(&render(&searched, 160, 30));
    assert!(searched_text.contains("/needle 1/2"), "{searched_text}");
    assert!(
        searched_text
            .matches("Moved from src/old.rs to dst/new.rs.")
            .count()
            == 2,
        "a pure move keeps its empty state while searched: {searched_text}"
    );
    assert!(!searched_text.contains("@@"), "{searched_text}");

    let next = press(&searched, &[Key::Char('n')]);
    assert_eq!(search_of(&next).cursor, 1, "n advances to the new side");
    assert_eq!(
        diff_ref(&next).body_scroll(),
        0,
        "a pure move has no visual rows to scroll"
    );
    let next_text = buffer_text(&render(&next, 160, 30));
    assert!(next_text.contains("/needle 2/2"), "{next_text}");
    assert!(!next_text.contains("@@"), "{next_text}");

    let wrapped = press(&next, &[Key::Char('n')]);
    assert_eq!(
        search_of(&wrapped).cursor,
        0,
        "n wraps back to the old side"
    );
}

#[test]
fn edited_move_search_reveals_the_old_then_the_new_side() {
    let (old, new) = source_lines(10, 85);
    let app = diff_app(vec![moved_file("src/old.rs", "lib/new.rs", &old, &new)]);

    assert!(
        diff_ref(&app).visual_row_for(SearchPane::Old, 10).is_some(),
        "the old match line is in the first hunk"
    );
    assert!(
        diff_ref(&app).visual_row_for(SearchPane::New, 85).is_some(),
        "the new match line is in the second hunk"
    );
    assert_eq!(diff_ref(&app).body_scroll(), 0);

    let searched = search_needle(&app);
    let search = search_of(&searched);
    assert_eq!(search.matches.len(), 2, "both sides of the move match");
    assert_eq!(
        (search.matches[0].pane, search.matches[0].line),
        (SearchPane::Old, 10)
    );
    assert_eq!(
        (search.matches[1].pane, search.matches[1].line),
        (SearchPane::New, 85)
    );
    assert_eq!(search.cursor, 0);
    assert_eq!(
        diff_ref(&searched).body_scroll(),
        0,
        "the old match is already visible"
    );

    let next = press(&searched, &[Key::Char('n')]);
    assert_eq!(search_of(&next).cursor, 1, "n advances to the new side");
    let scrolled = diff_ref(&next).body_scroll();
    assert!(
        scrolled > 0,
        "revealing the new-side match scrolls the diff body"
    );
    let next_text = buffer_text(&render(&next, 160, 30));
    assert!(next_text.contains("/needle 2/2"), "{next_text}");
    assert!(
        next_text.contains("needle new"),
        "the revealed new-side line is visible: {next_text}"
    );

    let back = press(&next, &[Key::Char('N')]);
    assert_eq!(search_of(&back).cursor, 0, "N returns to the old side");
    assert!(
        diff_ref(&back).body_scroll() < scrolled,
        "revealing the old-side match scrolls back up"
    );
}

#[test]
fn a_move_losing_its_only_projection_line_shows_a_hunk_and_searches_the_old_side() {
    let target = RepoPath::new("dst/new.rs").unwrap();
    let app = diff_app(vec![moved_file(
        "src/old.rs",
        "dst/new.rs",
        "needle\nfn t() {}\n",
        "",
    )]);

    assert_eq!(selected(&app).as_deref(), Some("dst/new.rs"));
    assert_eq!(file_row_count(&app), 1);
    assert_eq!(diff_ref(&app).diffs().len(), 1);
    assert!(
        !diff_ref(&app).rows().is_empty(),
        "a one-empty-side move keeps its focused rows"
    );
    assert!(
        diff_ref(&app)
            .rows()
            .iter()
            .all(|row| row.new_number.is_none()),
        "the target projection is empty"
    );

    let text = buffer_text(&render(&app, 160, 30));
    assert!(text.contains("BASE · src/old.rs"), "{text}");
    assert!(text.contains("TARGET · dst/new.rs"), "{text}");
    assert!(text.contains("@@"), "{text}");
    assert!(text.contains("needle"), "{text}");
    assert!(
        !text.contains("Moved from src/old.rs to dst/new.rs."),
        "a move with visible rows must not render the pure-move empty state: {text}"
    );
    assert!(!text.contains("No text changes"), "{text}");
    assert!(
        text.contains("dst/new.rs +0 −2"),
        "losing the projected lines counts each removed line once: {text}"
    );

    let searched = search_needle(&app);
    let search = search_of(&searched);
    assert_eq!(search.matches.len(), 1);
    assert_eq!(search.matches[0].pane, SearchPane::Old);
    assert_eq!(search.matches[0].line, 1);
    assert!(
        diff_ref(&searched)
            .visual_row_for(SearchPane::Old, 1)
            .is_some()
    );
    assert_eq!(
        app.workspace.files.moves.get(&target),
        Some(&RepoPath::new("src/old.rs").unwrap())
    );
}

#[test]
fn a_move_gaining_its_only_projection_line_shows_a_hunk_and_searches_the_new_side() {
    let app = diff_app(vec![moved_file(
        "src/old.rs",
        "dst/new.rs",
        "",
        "needle\nfn t() {}\n",
    )]);

    assert!(
        diff_ref(&app)
            .rows()
            .iter()
            .all(|row| row.old_number.is_none()),
        "the source projection is empty"
    );
    let text = buffer_text(&render(&app, 160, 30));
    assert!(text.contains("BASE · src/old.rs"), "{text}");
    assert!(text.contains("TARGET · dst/new.rs"), "{text}");
    assert!(text.contains("@@"), "{text}");
    assert!(
        !text.contains("Moved from src/old.rs to dst/new.rs."),
        "a visible one-sided move is not a pure-move empty state: {text}"
    );
    assert!(
        text.contains("dst/new.rs +2 −0"),
        "gaining the projected lines counts each added line once: {text}"
    );

    let searched = search_needle(&app);
    let search = search_of(&searched);
    assert_eq!(search.matches.len(), 1);
    assert_eq!(search.matches[0].pane, SearchPane::New);
    assert_eq!(search.matches[0].line, 1);
    assert!(
        diff_ref(&searched)
            .visual_row_for(SearchPane::New, 1)
            .is_some()
    );
}

#[test]
fn history_view_renders_a_pure_move_once_with_path_titles_and_the_move_empty_state() {
    let app = commits_app_with(
        3,
        vec![moved_file(
            "src/old.rs",
            "dst/new.rs",
            "fn a() {}\n",
            "fn a() {}\n",
        )],
    );

    assert_eq!(selected(&app).as_deref(), Some("dst/new.rs"));
    assert_eq!(file_row_count(&app), 1);
    assert_eq!(diff_ref(&app).diffs().len(), 1);

    let text = buffer_text(&render(&app, 120, 30));
    assert!(text.contains("BASE · src/old.rs"), "{text}");
    assert!(text.contains("TARGET · dst/new.rs"), "{text}");
    assert!(
        text.contains("src/old.rs → new.rs R"),
        "the history tree keeps the target-keyed row and badge: {text}"
    );
    assert_eq!(
        text.matches("Moved from src/old.rs to dst/new.rs.").count(),
        2,
        "both panes state the move in the history view: {text}"
    );
    assert!(!text.contains("@@"), "the pure move has no hunks: {text}");
    assert!(!text.contains("No text changes"), "{text}");
}

#[test]
fn range_reload_with_a_scope_keeps_the_target_key_and_requested_label() {
    let old_path = RepoPath::new("src/old.rs").unwrap();
    let target = RepoPath::new("dst/kept.rs").unwrap();
    let selection = Selection::new(vec![SelectionGroup::Path(old_path)]);
    let request = DiffRequest {
        selection,
        ..diff_request()
    };
    let app = base_app(LoadRequest::Diff {
        base: request.base.clone(),
        target: request.target.clone(),
        mode: request.mode,
        selection: request.selection.clone(),
        view: DiffView::Range,
        merge_base: false,
    });
    let (loaded, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request: request.clone(),
            result: Ok(vec![moved_file(
                "src/old.rs",
                "dst/kept.rs",
                "one\ntwo\n",
                "one\nTWO\n",
            )]
            .into()),
        }),
        &app,
    );
    let loaded = settle(loaded);
    assert_eq!(selected(&loaded).as_deref(), Some("dst/kept.rs"));

    let (next, _) = update(
        diff_msg(DiffMsg::DiffLoaded {
            request,
            result: Ok(vec![moved_file(
                "other/old.rs",
                "dst/kept.rs",
                "one\ntwo\n",
                "one\nTWO\n",
            )]
            .into()),
        }),
        &loaded,
    );
    let next = settle(next);

    assert_eq!(
        selected(&next).as_deref(),
        Some("dst/kept.rs"),
        "the target key survives a range reload"
    );
    assert_eq!(
        next.workspace.query.scope.label, "src/old.rs",
        "the reload keeps the user's requested scope label"
    );
    assert_eq!(
        next.workspace.files.moves.get(&target),
        Some(&RepoPath::new("other/old.rs").unwrap()),
        "the source label follows the newly loaded move"
    );
    assert!(
        tree_labels(&next).contains(&"other/old.rs → kept.rs"),
        "the tree refreshes the source label: {:?}",
        tree_labels(&next)
    );
    let text = buffer_text(&render(&next, 160, 30));
    assert!(text.contains("scope: src/old.rs"), "{text}");
    assert!(text.contains("BASE · other/old.rs"), "{text}");
    assert!(text.contains("TARGET · dst/kept.rs"), "{text}");
}

#[test]
fn history_reload_keeps_a_matching_target_and_falls_back_to_a_new_target() {
    let app = commits_app_with(
        3,
        vec![moved_file(
            "src/old.rs",
            "dst/kept.rs",
            "one\ntwo\n",
            "one\nTWO\n",
        )],
    );
    assert_eq!(selected(&app).as_deref(), Some("dst/kept.rs"));

    let (reloaded, _) = update(
        diff_msg(DiffMsg::HistoryLoaded {
            request: commits_request(),
            result: Ok((
                commit_steps(3).into(),
                vec![moved_file(
                    "other/old.rs",
                    "dst/kept.rs",
                    "one\ntwo\n",
                    "one\nTWO\n",
                )]
                .into(),
            )),
        }),
        &app,
    );
    let reloaded = settle(reloaded);
    assert_eq!(
        selected(&reloaded).as_deref(),
        Some("dst/kept.rs"),
        "a matching target key survives a history reload"
    );
    assert_eq!(
        reloaded
            .workspace
            .files
            .moves
            .get(&RepoPath::new("dst/kept.rs").unwrap()),
        Some(&RepoPath::new("other/old.rs").unwrap())
    );
    assert!(
        tree_labels(&reloaded).contains(&"other/old.rs → kept.rs"),
        "{:?}",
        tree_labels(&reloaded)
    );

    let (changed, _) = update(
        diff_msg(DiffMsg::HistoryLoaded {
            request: commits_request(),
            result: Ok((
                commit_steps(3).into(),
                vec![moved_file(
                    "other/old.rs",
                    "dst/second.rs",
                    "one\ntwo\n",
                    "one\nTWO\n",
                )]
                .into(),
            )),
        }),
        &reloaded,
    );
    let changed = settle(changed);
    assert_eq!(
        selected(&changed).as_deref(),
        Some("dst/second.rs"),
        "a vanished target falls back to the newly loaded target key"
    );
    assert_eq!(changed.workspace.files.paths.len(), 1);
    assert_eq!(file_row_count(&changed), 1);
    assert!(
        tree_labels(&changed).contains(&"other/old.rs → second.rs"),
        "{:?}",
        tree_labels(&changed)
    );
}

#[test]
fn history_step_reload_refreshes_the_source_and_resets_the_body() {
    let target = RepoPath::new("dst/kept.rs").unwrap();
    let mut app = commits_app_with(
        3,
        vec![moved_file(
            "src/old.rs",
            "dst/kept.rs",
            "one\ntwo\n",
            "one\nTWO\n",
        )],
    );
    app.plugins.diff.set_body_scroll(1);

    let (next, _) = update(
        diff_msg(DiffMsg::StepLoaded {
            index: 1,
            result: Ok(vec![moved_file(
                "other/old.rs",
                "dst/kept.rs",
                "one\ntwo\n",
                "one\nTWO\n",
            )]
            .into()),
        }),
        &app,
    );
    let next = settle(next);

    assert_eq!(
        selected(&next).as_deref(),
        Some("dst/kept.rs"),
        "the target key survives a history step reload"
    );
    assert_eq!(
        diff_ref(&next).body_scroll(),
        0,
        "a history step reload resets the diff body"
    );
    assert_eq!(
        next.workspace.files.moves.get(&target),
        Some(&RepoPath::new("other/old.rs").unwrap())
    );
    assert!(
        tree_labels(&next).contains(&"other/old.rs → kept.rs"),
        "{:?}",
        tree_labels(&next)
    );
    assert_eq!(diff_ref(&next).diffs().len(), 1);

    let (changed, _) = update(
        diff_msg(DiffMsg::StepLoaded {
            index: 2,
            result: Ok(vec![moved_file(
                "third/old.rs",
                "dst/second.rs",
                "one\ntwo\n",
                "one\nTWO\n",
            )]
            .into()),
        }),
        &next,
    );
    let changed = settle(changed);
    assert_eq!(
        selected(&changed).as_deref(),
        Some("dst/second.rs"),
        "a step whose move changed its target selects the new target key"
    );
    assert_eq!(file_row_count(&changed), 1);
    assert!(
        tree_labels(&changed).contains(&"third/old.rs → second.rs"),
        "{:?}",
        tree_labels(&changed)
    );
}

const LONG_MOVE_OLD: &str = "src/very/deep/directories/old_name.rs";
const LONG_MOVE_NEW: &str = "dst/renamed.rs";

fn long_move_app() -> App {
    diff_app(vec![moved_file(
        LONG_MOVE_OLD,
        LONG_MOVE_NEW,
        "fn a() {}\n",
        "fn a() {}\n",
    )])
}

#[test]
fn a_long_move_keeps_the_target_basename_in_the_tree_and_titles_at_80_and_120() {
    let app = long_move_app();
    assert_eq!(file_row_count(&app), 1);

    for width in [80_u16, 120_u16] {
        let buffer = render(&app, width, 24);
        let text = buffer_text(&buffer);
        let move_row = text
            .lines()
            .find(|line| line.contains('→'))
            .unwrap_or_else(|| panic!("width {width}: no move row in {text}"));
        assert!(
            move_row.contains("renamed.rs"),
            "width {width}: the target basename must survive the tree row: {move_row}"
        );
        if width == 80 {
            assert!(
                !move_row.contains("old_name.rs"),
                "the source basename must be fitted away before the target: {move_row}"
            );
        }

        let title = buffer_row(&buffer, 1);
        let (base, target) = title
            .split_once("TARGET")
            .unwrap_or_else(|| panic!("width {width}: no target title in {title}"));
        assert!(
            base.contains("BASE · ") && base.contains("old_name.rs"),
            "width {width}: the source title lost its basename: {title}"
        );
        assert!(
            target.contains("renamed.rs"),
            "width {width}: the target title lost its basename: {title}"
        );
        assert!(
            !text.contains("@@"),
            "width {width}: a pure move has no hunks: {text}"
        );
    }

    // At the wider layout the tree still has room for the source basename as
    // well as the always-prioritized target basename.
    let wide = buffer_text(&render(&app, 120, 24));
    let wide_row = wide
        .lines()
        .find(|line| line.contains('→'))
        .expect("the wide move row is visible");
    assert!(
        wide_row.contains("old_name.rs") && wide_row.contains("renamed.rs"),
        "the wide tree row must keep both basenames: {wide_row}"
    );
}

#[test]
fn a_long_pure_move_wraps_from_and_to_in_both_panes_at_80_and_120() {
    let app = long_move_app();

    for width in [80_u16, 120_u16] {
        let text = buffer_text(&render(&app, width, 24));
        let from = text
            .lines()
            .find(|line| line.contains("from ") && line.contains("old_name.rs"))
            .unwrap_or_else(|| panic!("width {width}: no from line in {text}"));
        let to = text
            .lines()
            .find(|line| line.contains("to ") && line.contains("renamed.rs"))
            .unwrap_or_else(|| panic!("width {width}: no to line in {text}"));

        assert!(
            from.matches("from ").count() >= 2,
            "width {width}: both panes must show from: {from}"
        );
        assert!(
            to.matches("to ").count() >= 2,
            "width {width}: both panes must show to: {to}"
        );
        assert!(
            from.matches("old_name.rs").count() >= 2,
            "width {width}: both panes must keep the source basename: {from}"
        );
        assert!(
            to.matches("renamed.rs").count() >= 2,
            "width {width}: both panes must keep the target basename: {to}"
        );
        assert!(
            !text.contains("Moved from src/very/deep/directories/old_name.rs to"),
            "width {width}: the long one-line detail must wrap instead of clip: {text}"
        );
        assert!(
            !text.contains("@@"),
            "width {width}: a pure move has no hunk rows: {text}"
        );
    }
}

fn long_search_text() -> String {
    let chunk = "0123456789abcdefghijklmnopqrstuvwxyz";
    format!("{chunk}{chunk}needle{chunk}{chunk}\n")
}

#[test]
fn a_long_pure_move_search_shows_the_side_and_highlights_the_excerpt_at_80_and_120() {
    let text = long_search_text();
    let app = diff_app(vec![moved_file(LONG_MOVE_OLD, LONG_MOVE_NEW, &text, &text)]);
    let searched = search_needle(&app);
    assert_eq!(search_of(&searched).matches.len(), 2);
    assert!(
        diff_ref(&searched).rows().is_empty(),
        "a pure move must not fabricate hunk rows"
    );

    let theme = Theme::dark();
    let match_background = theme.color(theme.palette.match_current_bg);
    for width in [80_u16, 120_u16] {
        let buffer = render(&searched, width, 24);
        let rendered = buffer_text(&buffer);
        assert!(
            rendered.contains("search old line 1 · 1/2"),
            "width {width}: missing old-side search feedback: {rendered}"
        );
        assert!(
            rendered.matches("needle").count() >= 2,
            "width {width}: both panes must show the canonical excerpt: {rendered}"
        );
        assert!(
            highlighted_occurrences(&buffer, "needle", match_background) >= 2,
            "width {width}: the current excerpt must be highlighted in both panes: {rendered}"
        );
        assert!(
            !rendered.contains("@@"),
            "width {width}: search feedback must not create hunks: {rendered}"
        );
    }

    let next = press(&searched, &[Key::Char('n')]);
    assert_eq!(search_of(&next).cursor, 1);
    for width in [80_u16, 120_u16] {
        let buffer = render(&next, width, 24);
        let rendered = buffer_text(&buffer);
        assert!(
            rendered.contains("search new line 1 · 2/2"),
            "width {width}: missing new-side search feedback: {rendered}"
        );
        assert!(
            highlighted_occurrences(&buffer, "needle", match_background) >= 2,
            "width {width}: the new-side excerpt must stay highlighted: {rendered}"
        );
        assert!(
            !rendered.contains("@@"),
            "width {width}: the new-side hit must not create hunks: {rendered}"
        );
    }

    let back = press(&next, &[Key::Char('N')]);
    assert_eq!(search_of(&back).cursor, 0);
    let back_text = buffer_text(&render(&back, 80, 24));
    assert!(
        back_text.contains("search old line 1 · 1/2"),
        "N must return to the old-side feedback: {back_text}"
    );
}
