//! The request vocabulary: the kernel effects pages and overlays ask for.
//!
//! This is the in-crate form of the plugin plan's section 4.6 `Request` enum,
//! minus the overlay, prompt, and view variants that land in 4.x. The
//! [`Request`] values are applied depth-first by [`crate::app::apply_request`].
//!
//! Some of the payload types are stand-ins for types that arrive in later
//! tasks:
//!
//! - [`FileSet`] is paths-only until task 2.2 adds badges and empty text.
//! - [`Query`] and [`QueryChange`] stand in for `workspace::Query` until 2.3.
//! - [`NavigatorResize`] is the eventual resize request.
//! - [`Focus`] is today's normalized focus until 3.1/3.2 introduce `FocusKey`.
//!
//! The vocabulary is wired into the request queue across tasks 2.2/2.3/3.x, so
//! several variants are constructed only by tests until then. One allowance
//! here covers them without per-variant noise.
#![allow(dead_code)]

use std::sync::Arc;

use base::{ProjectionMode, RepoPath, Selection};

use crate::command::CommandId;
use crate::page::Scope;
use crate::render::metrics::Focus;

/// A navigator width change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavigatorResize {
    Wider,
    Narrower,
    Reset,
}

/// The file set the workspace holds. 2.1 carries only the paths; task 2.2 adds
/// badges and the empty text.
#[derive(Clone, Debug)]
pub(crate) struct FileSet {
    pub paths: Arc<[RepoPath]>,
}

/// The committed projection query. Task 2.3 moves it into the workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Query {
    pub mode: ProjectionMode,
    pub scope: Scope,
}

/// A query change requested by the mode picker or the scope chooser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum QueryChange {
    Mode(ProjectionMode),
    Scope(Selection),
}

/// A kernel request. Depth-first application is in [`crate::app`].
#[derive(Clone, Debug)]
pub(crate) enum Request {
    Quit,
    Diagnose(String),
    ClearDiagnostic,
    SetFiles(FileSet),
    Select(Option<RepoPath>),
    Requery(QueryChange),
    QueryLoaded(Query),
    SetSearch(String),
    CommitSearch,
    ClearSearch,
    Focus(Focus),
    FocusDefault,
    CycleFocus { forward: bool },
    ResizeNavigator(NavigatorResize),
    CancelPrefix,
    RunCommand(CommandId),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base::{ProjectionMode, RepoPath};

    use super::*;
    use crate::api::Cmd;
    use crate::app::{App, Diagnostic, apply_request, fold};
    use crate::command::{CommandId, Owner};
    use crate::component::overlay::Overlay;
    use crate::page::{Loaded, OutMsg};
    use crate::render::metrics::Focus;
    use crate::route::{DiffRequest, ShowRequest};
    use crate::test_support::{
        diff_app, diff_request, file_diff, folder_app, show_app, show_ref, show_request,
        source_outline, two_files,
    };

    fn run(app: &mut App, request: Request) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        apply_request(app, request, &mut cmds);
        cmds
    }

    #[test]
    fn quit_sets_the_flag() {
        let mut app = two_files();
        let cmds = run(&mut app, Request::Quit);
        assert!(app.chrome.quit);
        assert!(cmds.is_empty());
    }

    #[test]
    fn diagnose_sets_a_forty_tick_toast() {
        let mut app = two_files();
        run(&mut app, Request::Diagnose("boom".to_owned()));
        let diagnostic = app.diagnostic.as_ref().expect("a toast");
        assert_eq!(diagnostic.text, "boom");
        assert_eq!(diagnostic.ttl, 40);
    }

    #[test]
    fn clear_diagnostic_clears_it() {
        let mut app = two_files();
        app.diagnostic = Some(Diagnostic {
            text: "x".to_owned(),
            ttl: 3,
        });
        run(&mut app, Request::ClearDiagnostic);
        assert!(app.diagnostic.is_none());
    }

    #[test]
    fn set_files_drops_a_missing_selection_without_an_event() {
        let mut app = two_files();
        assert_eq!(app.loaded.selected(), Some(&RepoPath::new("a.rs").unwrap()));

        let set = FileSet {
            paths: Arc::from(vec![RepoPath::new("b.rs").unwrap()]),
        };
        let cmds = run(&mut app, Request::SetFiles(set));

        assert!(app.loaded.selected().is_none());
        assert!(cmds.is_empty());
        assert!(app.diagnostic.is_none(), "no event fires in 2.1");
    }

    #[test]
    fn select_always_notifies_even_for_the_same_path() {
        let mut app = folder_app();
        let path = app.loaded.selected().cloned().expect("a selection");
        if let Loaded::Show(show) = &mut app.loaded {
            show.body.scroll = 3;
            show.body.hscroll = 2;
        }

        run(&mut app, Request::Select(Some(path)));

        if let Loaded::Show(show) = &app.loaded {
            assert_eq!(show.body.scroll, 0, "re-selecting resets the body scroll");
            assert_eq!(show.body.hscroll, 0);
        } else {
            panic!("expected a show");
        }
    }

    #[test]
    fn select_clears_a_pending_prefix() {
        let mut app = folder_app();
        app.keymap.pending = Some('z');
        app.overlay = Some(Overlay::which_key());
        let path = app.loaded.selected().cloned().expect("a selection");

        run(&mut app, Request::Select(Some(path)));

        assert!(app.keymap.pending.is_none());
        assert!(app.overlay.is_none(), "the which-key band closes");
    }

    #[test]
    fn requery_reloads_the_active_view() {
        let mut show = folder_app();
        let show_cmds = run(
            &mut show,
            Request::Requery(QueryChange::Mode(ProjectionMode::Signatures)),
        );
        assert_eq!(
            show_cmds,
            vec![Cmd::Show(ShowRequest {
                mode: ProjectionMode::Signatures,
                ..show_request()
            })]
        );

        let mut diff = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let diff_cmds = run(
            &mut diff,
            Request::Requery(QueryChange::Mode(ProjectionMode::Tests)),
        );
        assert_eq!(
            diff_cmds,
            vec![Cmd::Diff(DiffRequest {
                mode: ProjectionMode::Tests,
                ..diff_request()
            })]
        );
    }

    #[test]
    fn resize_navigator_clamps_to_15_60_step_5() {
        let mut app = two_files();
        assert_eq!(app.chrome.tree_percent, 30);

        run(&mut app, Request::ResizeNavigator(NavigatorResize::Wider));
        assert_eq!(app.chrome.tree_percent, 35);

        app.chrome.tree_percent = 55;
        run(&mut app, Request::ResizeNavigator(NavigatorResize::Wider));
        assert_eq!(app.chrome.tree_percent, 60);

        app.chrome.tree_percent = 60;
        run(&mut app, Request::ResizeNavigator(NavigatorResize::Wider));
        assert_eq!(app.chrome.tree_percent, 60, "clamped at the maximum");

        app.chrome.tree_percent = 30;
        run(
            &mut app,
            Request::ResizeNavigator(NavigatorResize::Narrower),
        );
        assert_eq!(app.chrome.tree_percent, 25);

        app.chrome.tree_percent = 20;
        run(
            &mut app,
            Request::ResizeNavigator(NavigatorResize::Narrower),
        );
        assert_eq!(app.chrome.tree_percent, 15);

        app.chrome.tree_percent = 15;
        run(
            &mut app,
            Request::ResizeNavigator(NavigatorResize::Narrower),
        );
        assert_eq!(app.chrome.tree_percent, 15, "clamped at the minimum");

        app.chrome.tree_percent = 45;
        run(&mut app, Request::ResizeNavigator(NavigatorResize::Reset));
        assert_eq!(app.chrome.tree_percent, 30);
    }

    #[test]
    fn set_search_collects_matches_with_cursor_zero() {
        let mut app = show_app(vec![source_outline("a.rs", "alpha\nbeta\nalpha again\n")]);
        run(&mut app, Request::SetSearch("alpha".to_owned()));
        let search = app.loaded.search().expect("a search");
        assert_eq!(search.matches.len(), 2);
        assert_eq!(search.cursor, 0);
    }

    #[test]
    fn set_search_empty_clears() {
        let mut app = show_app(vec![source_outline("a.rs", "alpha\n")]);
        run(&mut app, Request::SetSearch("alpha".to_owned()));
        assert!(app.loaded.search().is_some());
        run(&mut app, Request::SetSearch(String::new()));
        assert!(app.loaded.search().is_none());
    }

    #[test]
    fn commit_search_reveals_the_current_match() {
        let source: String = (1..=40).map(|n| format!("line {n}\n")).collect();
        let mut app = show_app(vec![source_outline("a.rs", &(source + "needle\n"))]);
        run(&mut app, Request::SetSearch("needle".to_owned()));
        assert_eq!(show_ref(&app).body.cursor_line, 1, "search does not scroll");

        run(&mut app, Request::CommitSearch);
        assert_eq!(show_ref(&app).body.cursor_line, 41);
    }

    #[test]
    fn clear_search_clears() {
        let mut app = show_app(vec![source_outline("a.rs", "alpha\n")]);
        run(&mut app, Request::SetSearch("alpha".to_owned()));
        assert!(app.loaded.search().is_some());
        run(&mut app, Request::ClearSearch);
        assert!(app.loaded.search().is_none());
    }

    #[test]
    fn cancel_prefix_clears_the_band() {
        let mut app = folder_app();
        run(&mut app, Request::CancelPrefix);
        assert!(app.keymap.pending.is_none(), "no pending prefix is a no-op");

        app.keymap.pending = Some('z');
        app.overlay = Some(Overlay::which_key());
        run(&mut app, Request::CancelPrefix);
        assert!(app.keymap.pending.is_none());
        assert!(app.overlay.is_none());
    }

    #[test]
    fn run_command_runs_a_registered_command() {
        let mut app = folder_app();
        assert_eq!(app.chrome.tree_percent, 30);
        run(
            &mut app,
            Request::RunCommand(CommandId {
                owner: Owner::Tree,
                name: "wider",
            }),
        );
        assert_eq!(app.chrome.tree_percent, 35);
    }

    #[test]
    fn focus_and_cycle_focus_move_the_focus() {
        let mut app = folder_app();
        assert_eq!(app.loaded.focus(), Focus::Tree);

        run(&mut app, Request::CycleFocus { forward: true });
        assert_eq!(app.loaded.focus(), Focus::Content);

        run(&mut app, Request::Focus(Focus::Tree));
        assert_eq!(app.loaded.focus(), Focus::Tree);

        run(&mut app, Request::Focus(Focus::Content));
        run(&mut app, Request::FocusDefault);
        assert_eq!(app.loaded.focus(), Focus::Tree, "the navigator is default");
    }

    #[test]
    fn fold_maps_request_diagnose_to_the_toast() {
        let mut app = folder_app();
        let mut cmds = Vec::new();
        fold(
            &mut app,
            vec![OutMsg::Request(Request::Diagnose("nope".to_owned()))],
            &mut cmds,
        );
        assert_eq!(
            app.diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.text.as_str()),
            Some("nope")
        );
        assert!(cmds.is_empty());
    }

    #[test]
    fn fold_keeps_effect_order() {
        let first = Cmd::Show(show_request());
        let second = Cmd::Diff(diff_request());
        let mut app = folder_app();
        let mut cmds = Vec::new();
        fold(
            &mut app,
            vec![
                OutMsg::Effect(first.clone()),
                OutMsg::Request(Request::Diagnose("boom".to_owned())),
                OutMsg::Effect(second.clone()),
            ],
            &mut cmds,
        );
        assert_eq!(cmds, vec![first, second]);
        assert_eq!(
            app.diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.text.as_str()),
            Some("boom")
        );
    }
}
