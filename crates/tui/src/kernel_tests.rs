//! Kernel tests migrated from `request.rs` and `lib.rs`, plus the kernel
//! boundary scan and a settle-oracle check.
//!
//! `#[cfg(test)]` only. These use the real registry through `test_support`, so
//! they live outside `src/kernel/`; the boundary scan below still proves the
//! kernel sources name no plugin.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use base::{ProjectionMode, RepoPath, Selection};

use crate::api::Cmd;
use crate::kernel::app::{Diagnostic, Msg, settle};
use crate::kernel::plugin::Request;
use crate::kernel::runtime::{Driver, Session, run_with, translate, translate_mouse};
use crate::plugins::PluginsMsg;
use crate::test_support::{
    App, Cmd as PluginsCmd, diff_app, file_diff, folder_app, render, show_app, show_ref,
    show_request, two_files,
};
use crate::util::input::{Key, Mouse, MouseKind};

fn legacy_cmd(cmd: PluginsCmd) -> Cmd {
    match cmd {
        crate::plugins::PluginsCmd::Legacy(cmd) => cmd,
        crate::plugins::PluginsCmd::Tree(never) => match never {},
        crate::plugins::PluginsCmd::WhichKey(never) => match never {},
    }
}

fn run(app: &mut App, request: Request) -> Vec<Cmd> {
    let specs = crate::kernel::app::collect_specs(app);
    let mut cmds = Vec::new();
    crate::kernel::dispatch::apply_request(app, request, &mut cmds, 0, &specs);
    cmds.into_iter().map(legacy_cmd).collect()
}

// -- request semantics ------------------------------------------------------

#[test]
fn quit_sets_the_flag() {
    let mut app = two_files();
    let cmds = run(&mut app, Request::Quit);
    assert!(app.kernel.chrome.quit);
    assert!(cmds.is_empty());
}

#[test]
fn diagnose_sets_a_forty_tick_toast() {
    let mut app = two_files();
    run(&mut app, Request::Diagnose("boom".to_owned()));
    let diagnostic = app.kernel.diagnostic.as_ref().expect("a toast");
    assert_eq!(diagnostic.text, "boom");
    assert_eq!(diagnostic.ttl, 40);
}

#[test]
fn clear_diagnostic_clears_it() {
    let mut app = two_files();
    app.kernel.diagnostic = Some(Diagnostic {
        text: "x".to_owned(),
        ttl: 3,
    });
    run(&mut app, Request::ClearDiagnostic);
    assert!(app.kernel.diagnostic.is_none());
}

#[test]
fn set_files_drops_a_missing_selection_without_an_event() {
    let mut app = two_files();
    assert_eq!(
        app.kernel.workspace.selected,
        Some(RepoPath::new("a.rs").unwrap())
    );

    let set = crate::kernel::workspace::FileSet {
        paths: Arc::from(Vec::new()),
        ..Default::default()
    };
    let cmds = run(&mut app, Request::SetFiles(set));

    assert!(app.kernel.workspace.selected.is_none());
    assert!(cmds.is_empty());
    assert!(app.kernel.diagnostic.is_none(), "no event fires");
}

#[test]
fn set_files_falls_back_to_the_nearest_visible() {
    let mut app = two_files();
    assert_eq!(
        app.kernel.workspace.selected,
        Some(RepoPath::new("a.rs").unwrap())
    );

    let set = crate::kernel::workspace::FileSet {
        paths: Arc::from(vec![RepoPath::new("b.rs").unwrap()]),
        ..Default::default()
    };
    run(&mut app, Request::SetFiles(set));

    assert_eq!(
        app.kernel.workspace.selected,
        Some(RepoPath::new("b.rs").unwrap())
    );
}

#[test]
fn set_files_does_not_reset_the_body_when_it_drops_a_missing_selection() {
    let mut app = folder_app();
    if let crate::page::Loaded::Show(show) = &mut app.plugins.legacy.loaded {
        show.body.scroll = 2;
        show.body.hscroll = 1;
    }

    let set = crate::kernel::workspace::FileSet {
        paths: Arc::from(Vec::new()),
        ..Default::default()
    };
    run(&mut app, Request::SetFiles(set));

    assert!(app.kernel.workspace.selected.is_none());
    if let crate::page::Loaded::Show(show) = &app.plugins.legacy.loaded {
        assert_eq!(
            show.body.scroll, 2,
            "dropping a selection must not reset the body"
        );
        assert_eq!(show.body.hscroll, 1);
    } else {
        panic!("expected a show");
    }
}

#[test]
fn select_always_notifies_even_for_the_same_path() {
    let mut app = folder_app();
    let path = app.kernel.workspace.selected.clone().expect("a selection");
    if let crate::page::Loaded::Show(show) = &mut app.plugins.legacy.loaded {
        show.body.scroll = 3;
        show.body.hscroll = 2;
    }

    run(&mut app, Request::Select(Some(path)));

    if let crate::page::Loaded::Show(show) = &app.plugins.legacy.loaded {
        assert_eq!(show.body.scroll, 0, "re-selecting resets the body scroll");
        assert_eq!(show.body.hscroll, 0);
    } else {
        panic!("expected a show");
    }
}

#[test]
fn select_clears_a_pending_prefix() {
    let mut app = folder_app();
    app.kernel.keymap.pending = Some('z');
    app.kernel.overlay = Some(crate::plugin::WHICH_KEY_PANE);
    let path = app.kernel.workspace.selected.clone().expect("a selection");

    run(&mut app, Request::Select(Some(path)));

    assert!(app.kernel.keymap.pending.is_none());
    assert!(app.kernel.overlay.is_none(), "the which-key band closes");
}

#[test]
fn requery_reloads_the_active_view() {
    let mut show = folder_app();
    let show_cmds = run(
        &mut show,
        Request::Requery(crate::kernel::plugin::QueryChange::Mode(
            ProjectionMode::Signatures,
        )),
    );
    assert_eq!(
        show_cmds,
        vec![Cmd::Show(crate::route::ShowRequest {
            mode: ProjectionMode::Signatures,
            ..show_request()
        })]
    );

    let mut diff = diff_app(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
    let diff_cmds = run(
        &mut diff,
        Request::Requery(crate::kernel::plugin::QueryChange::Mode(
            ProjectionMode::Tests,
        )),
    );
    assert_eq!(
        diff_cmds,
        vec![Cmd::Diff(crate::route::DiffRequest {
            mode: ProjectionMode::Tests,
            ..crate::test_support::diff_request()
        })]
    );
}

#[test]
fn resize_navigator_clamps_to_15_60_step_5() {
    let mut app = two_files();
    assert_eq!(app.kernel.chrome.tree_percent, 30);

    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Wider),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 35);

    app.kernel.chrome.tree_percent = 55;
    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Wider),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 60);

    app.kernel.chrome.tree_percent = 60;
    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Wider),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 60, "clamped at the maximum");

    app.kernel.chrome.tree_percent = 30;
    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Narrower),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 25);

    app.kernel.chrome.tree_percent = 20;
    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Narrower),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 15);

    app.kernel.chrome.tree_percent = 15;
    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Narrower),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 15, "clamped at the minimum");

    app.kernel.chrome.tree_percent = 45;
    run(
        &mut app,
        Request::ResizeNavigator(crate::kernel::plugin::NavigatorResize::Reset),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 30);
}

#[test]
fn set_search_collects_matches_with_cursor_zero() {
    let mut app = show_app(vec![crate::test_support::source_outline(
        "a.rs",
        "alpha\nbeta\nalpha again\n",
    )]);
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    let search = app.kernel.workspace.search.expect("a search");
    assert_eq!(search.matches.len(), 2);
    assert_eq!(search.cursor, 0);
}

#[test]
fn set_search_empty_clears() {
    let mut app = show_app(vec![crate::test_support::source_outline("a.rs", "alpha\n")]);
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    assert!(app.kernel.workspace.search.is_some());
    run(&mut app, Request::SetSearch(String::new()));
    assert!(app.kernel.workspace.search.is_none());
}

#[test]
fn commit_search_reveals_the_current_match() {
    let source: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    let mut app = show_app(vec![crate::test_support::source_outline(
        "a.rs",
        &(source + "needle\n"),
    )]);
    run(&mut app, Request::SetSearch("needle".to_owned()));
    assert_eq!(show_ref(&app).body.cursor_line, 1, "search does not scroll");

    run(&mut app, Request::CommitSearch);
    assert_eq!(show_ref(&app).body.cursor_line, 41);
}

#[test]
fn clear_search_clears() {
    let mut app = show_app(vec![crate::test_support::source_outline("a.rs", "alpha\n")]);
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    assert!(app.kernel.workspace.search.is_some());
    run(&mut app, Request::ClearSearch);
    assert!(app.kernel.workspace.search.is_none());
}

#[test]
fn cancel_prefix_clears_the_band() {
    let mut app = folder_app();
    run(&mut app, Request::CancelPrefix);
    assert!(
        app.kernel.keymap.pending.is_none(),
        "no pending prefix is a no-op"
    );

    app.kernel.keymap.pending = Some('z');
    app.kernel.overlay = Some(crate::plugin::WHICH_KEY_PANE);
    run(&mut app, Request::CancelPrefix);
    assert!(app.kernel.keymap.pending.is_none());
    assert!(app.kernel.overlay.is_none());
}

#[test]
fn open_overlay_replaces_and_close_is_targeted() {
    let mut app = folder_app();
    run(&mut app, Request::OpenOverlay(crate::plugin::SCOPE_PANE));
    assert_eq!(app.kernel.overlay, Some(crate::plugin::SCOPE_PANE));

    run(&mut app, Request::OpenOverlay(crate::plugin::HELP_PANE));
    assert_eq!(app.kernel.overlay, Some(crate::plugin::HELP_PANE));

    run(&mut app, Request::CloseOverlay(crate::plugin::SCOPE_PANE));
    assert_eq!(
        app.kernel.overlay,
        Some(crate::plugin::HELP_PANE),
        "closing another pane leaves the open overlay alone"
    );
    run(&mut app, Request::CloseOverlay(crate::plugin::HELP_PANE));
    assert_eq!(app.kernel.overlay, None);
}

#[test]
fn replacing_an_overlay_notifies_its_owner() {
    let mut app = folder_app();
    run(
        &mut app,
        Request::RunCommand(crate::kernel::command::CommandId {
            owner: crate::kernel::command::Owner::Overlay,
            name: "scope.open",
        }),
    );
    assert_eq!(app.kernel.overlay, Some(crate::plugin::SCOPE_PANE));
    assert!(matches!(
        app.plugins.legacy.overlay,
        Some(crate::component::overlay::Overlay::Scope(_))
    ));

    run(&mut app, Request::OpenOverlay(crate::plugin::HELP_PANE));
    assert_eq!(app.kernel.overlay, Some(crate::plugin::HELP_PANE));
    assert!(
        app.plugins.legacy.overlay.is_none(),
        "the replaced owner receives OverlayClosed"
    );
}

#[test]
fn run_command_runs_a_registered_command() {
    let mut app = folder_app();
    assert_eq!(app.kernel.chrome.tree_percent, 30);
    run(
        &mut app,
        Request::RunCommand(crate::kernel::command::CommandId {
            owner: crate::kernel::command::Owner::Tree,
            name: "wider",
        }),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 35);
}

#[test]
fn focus_and_cycle_focus_move_the_focus() {
    let mut app = folder_app();
    assert_eq!(app.kernel.focus, Some(crate::plugin::TREE_FOCUS));

    run(&mut app, Request::CycleFocus { forward: true });
    assert_eq!(app.kernel.focus, Some(crate::plugin::SOURCE_FOCUS));

    run(&mut app, Request::Focus(crate::plugin::TREE_FOCUS));
    assert_eq!(app.kernel.focus, Some(crate::plugin::TREE_FOCUS));

    run(&mut app, Request::Focus(crate::plugin::SOURCE_FOCUS));
    run(&mut app, Request::FocusDefault);
    assert_eq!(
        app.kernel.focus,
        Some(crate::plugin::TREE_FOCUS),
        "the navigator is default"
    );
}

// -- depth-first determinism ------------------------------------------------

#[test]
fn a_request_cascade_is_depth_first_and_deterministic() {
    let mut app = show_app(vec![crate::test_support::source_outline("a.rs", "a\n")]);
    // A `SetSearch` emits a match reveal through the plugin path and back; the
    // effects and the final cursor must be identical across runs.
    let mut first = app.clone();
    let mut second = app.clone();
    run(&mut first, Request::SetSearch("a".to_owned()));
    run(&mut second, Request::SetSearch("a".to_owned()));
    assert_eq!(
        first.kernel.workspace.search,
        second.kernel.workspace.search
    );
    app = first;
    assert!(app.kernel.workspace.search.is_some());
}

// -- settle oracle ----------------------------------------------------------

#[test]
fn settle_is_stable_and_resyncs_the_search() {
    let app = show_app(vec![crate::test_support::source_outline(
        "a.rs",
        "alpha\nbeta\nalpha\n",
    )]);
    let once = settle(app);
    let twice = settle(once.clone());
    assert_eq!(
        render(&once, 100, 30),
        render(&twice, 100, 30),
        "settle is idempotent",
    );
}

// -- runtime ----------------------------------------------------------------

#[derive(Default)]
struct Log {
    setup: Vec<&'static str>,
    teardown: Vec<&'static str>,
    draws: usize,
}

struct MockDriver {
    fail_at: Option<usize>,
    log: Rc<RefCell<Log>>,
    messages: VecDeque<Msg<PluginsMsg>>,
}

impl MockDriver {
    fn new(
        fail_at: Option<usize>,
        log: Rc<RefCell<Log>>,
        messages: VecDeque<Msg<PluginsMsg>>,
    ) -> Self {
        Self {
            fail_at,
            log,
            messages,
        }
    }
}

impl Driver<crate::plugins::Plugins> for MockDriver {
    fn setup(&mut self) -> Result<(), crate::TuiError> {
        for (index, step) in ["raw", "alternate", "hidden", "mouse"]
            .into_iter()
            .enumerate()
        {
            if self.fail_at == Some(index) {
                return Err(crate::TuiError::Terminal(std::io::Error::other(
                    "setup failed",
                )));
            }
            self.log.borrow_mut().setup.push(step);
        }
        Ok(())
    }

    fn teardown(&mut self) {
        let mut log = self.log.borrow_mut();
        if log.setup.contains(&"mouse") {
            log.teardown.push("mouse_off");
        }
        if log.setup.contains(&"hidden") {
            log.teardown.push("show_cursor");
        }
        if log.setup.contains(&"alternate") {
            log.teardown.push("leave_alternate");
        }
        if log.setup.contains(&"raw") {
            log.teardown.push("disable_raw");
        }
    }

    fn size(&mut self) -> Result<(u16, u16), crate::TuiError> {
        Ok((100, 30))
    }

    fn draw(&mut self, _model: &App) -> Result<(), crate::TuiError> {
        self.log.borrow_mut().draws += 1;
        Ok(())
    }

    fn read_msg(&mut self) -> Result<Msg<PluginsMsg>, crate::TuiError> {
        Ok(self
            .messages
            .pop_front()
            .unwrap_or(Msg::Key(Key::Char('q'))))
    }

    fn try_read_msg(&mut self) -> Result<Option<Msg<PluginsMsg>>, crate::TuiError> {
        Ok(self.messages.pop_front())
    }
}

fn log() -> Rc<RefCell<Log>> {
    Rc::new(RefCell::new(Log::default()))
}

fn session_app(revision: &str) -> (App, Vec<PluginsCmd>) {
    crate::plugins::build(crate::plugins::KernelOptions {
        root: "/repo".to_owned(),
        request: crate::route::LoadRequest::Show {
            revision: revision.to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
        },
        scope_label: "all".to_owned(),
        width: 100,
        height: 30,
        theme: crate::render::theme::Theme::dark(),
        icons: crate::render::icons::Icons::new(crate::render::icons::IconStyle::None),
    })
}

#[test]
fn a_setup_failure_tears_down_every_step_that_succeeded() {
    let cases: [(Option<usize>, Vec<&str>); 5] = [
        (Some(0), vec![]),
        (Some(1), vec!["disable_raw"]),
        (Some(2), vec!["leave_alternate", "disable_raw"]),
        (
            Some(3),
            vec!["show_cursor", "leave_alternate", "disable_raw"],
        ),
        (
            None,
            vec!["mouse_off", "show_cursor", "leave_alternate", "disable_raw"],
        ),
    ];

    for (fail_at, expected) in cases {
        let log = log();
        let mut session = Session::<MockDriver, crate::plugins::Plugins>::new(MockDriver::new(
            fail_at,
            log.clone(),
            VecDeque::new(),
        ));
        let result = session.setup();
        assert_eq!(result.is_err(), fail_at.is_some());
        drop(session);
        assert_eq!(log.borrow().teardown, expected, "fail_at = {fail_at:?}");
    }
}

#[test]
fn control_keys_translate_to_distinct_messages() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
    assert!(matches!(
        translate::<PluginsMsg>(ctrl(KeyCode::Char('p'))),
        Some(Msg::Key(Key::CtrlP))
    ));
    assert!(matches!(
        translate::<PluginsMsg>(ctrl(KeyCode::Char('j'))),
        Some(Msg::Key(Key::CtrlJ))
    ));
    assert!(matches!(
        translate::<PluginsMsg>(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
        Some(Msg::Key(Key::PageDown))
    ));
    assert!(matches!(
        translate::<PluginsMsg>(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
        Some(Msg::Key(Key::Char('p')))
    ));
}

#[test]
fn mouse_clicks_and_wheel_translate_but_motion_does_not() {
    use crossterm::event::{MouseButton, MouseEventKind};

    let at = |kind| crossterm::event::MouseEvent {
        kind,
        column: 7,
        row: 3,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };

    assert!(matches!(
        translate_mouse::<PluginsMsg>(at(MouseEventKind::Down(MouseButton::Left))),
        Some(Msg::Mouse(Mouse {
            column: 7,
            row: 3,
            kind: MouseKind::Click,
        }))
    ));
    assert!(matches!(
        translate_mouse::<PluginsMsg>(at(MouseEventKind::ScrollUp)),
        Some(Msg::Mouse(Mouse {
            kind: MouseKind::ScrollUp,
            ..
        }))
    ));
    assert!(translate_mouse::<PluginsMsg>(at(MouseEventKind::Moved)).is_none());
    assert!(translate_mouse::<PluginsMsg>(at(MouseEventKind::Up(MouseButton::Left))).is_none());
}

#[test]
fn a_run_quits_on_q_and_restores_the_terminal() {
    let repo = TestRepo::new();
    let engine = engine::Engine::discover(repo.path()).expect("discover");
    let log = log();
    let messages = VecDeque::from([Msg::Key(Key::Char('q'))]);
    let driver = MockDriver::new(None, log.clone(), messages);
    let (app, startup) = session_app("HEAD");

    run_with(&engine, app, startup, driver).expect("run");

    let log = log.borrow();
    assert_eq!(log.setup, vec!["raw", "alternate", "hidden", "mouse"]);
    assert_eq!(
        log.teardown,
        vec!["mouse_off", "show_cursor", "leave_alternate", "disable_raw"]
    );
    assert!(log.draws >= 1, "the model must be drawn at least once");
}

#[test]
fn a_burst_of_input_is_folded_into_one_frame() {
    let repo = TestRepo::new();
    let engine = engine::Engine::discover(repo.path()).expect("discover");
    let log = log();
    let messages = VecDeque::from([
        Msg::Key(Key::Down),
        Msg::Key(Key::Down),
        Msg::Key(Key::Down),
        Msg::Key(Key::Char('q')),
    ]);
    let driver = MockDriver::new(None, log.clone(), messages);
    let (app, startup) = session_app("HEAD");

    run_with(&engine, app, startup, driver).expect("run");

    assert_eq!(log.borrow().draws, 2);
}

#[test]
fn a_failing_initial_projection_restores_the_terminal_and_errors() {
    let repo = TestRepo::new();
    let engine = engine::Engine::discover(repo.path()).expect("discover");
    let log = log();
    let driver = MockDriver::new(None, log.clone(), VecDeque::new());
    let (app, startup) = session_app("no-such-revision");

    let error = run_with(&engine, app, startup, driver).expect_err("startup failure");
    assert!(matches!(error, crate::TuiError::Engine(_)), "got {error:?}");
    assert_eq!(
        log.borrow().teardown,
        vec!["mouse_off", "show_cursor", "leave_alternate", "disable_raw"]
    );
}

// -- boundary scan ----------------------------------------------------------

#[test]
fn the_kernel_sources_name_no_plugin() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src/kernel");
    let mut files = Vec::new();
    collect_rs(std::path::Path::new(root), &mut files);
    assert!(!files.is_empty(), "the kernel must have sources");

    let needles: Vec<String> = vec![
        ["plugins", "::"].concat(),
        ["crate::", "plugins"].concat(),
        ["pa", "ge::"].concat(),
        ["component", "::"].concat(),
        ["Load", "ed"].concat(),
        ["Show", "Request"].concat(),
        ["Diff", "Request"].concat(),
        ["History", "Payload"].concat(),
        ["Visual", "Row"].concat(),
        ["layout", "_diff"].concat(),
        ["commit", "_offset"].concat(),
        ["gutter", "_width"].concat(),
        ["enum Over", "lay"].concat(),
        ["struct ", "Tree"].concat(),
    ];

    for file in &files {
        let text = std::fs::read_to_string(file).expect("read a kernel source");
        for needle in &needles {
            if needle.contains("::") || needle.contains(' ') {
                assert!(
                    !text.contains(needle.as_str()),
                    "{} contains the leak {needle:?}",
                    file.display()
                );
            } else {
                assert!(
                    !contains_word(&text, needle),
                    "{} contains the leak {needle:?}",
                    file.display()
                );
            }
        }
    }
}

/// Whether `text` contains `word` as a whole identifier (not a substring of a
/// longer one, so `QueryLoaded` does not match `Loaded`).
fn contains_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(index) = text[from..].find(word) {
        let start = from + index;
        let end = start + word.len();
        let before_ok = start == 0 || !is_word_byte(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_word_byte(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = start + word.len();
    }
    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read the kernel directory") {
        let entry = entry.expect("a directory entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

// -- helpers ----------------------------------------------------------------

struct TestRepo {
    dir: tempfile::TempDir,
}

impl TestRepo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary directory");
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "init"]);
        Self { dir }
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Codect Test",
            "-c",
            "user.email=codect@example.invalid",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00+0000")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00+0000")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}
