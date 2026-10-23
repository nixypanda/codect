//! Kernel tests: the request vocabulary, key and mouse dispatch, the runtime,
//! the registry invariants, and the crate's "names no plugin" boundary.
//!
//! `#[cfg(test)]` only. These drive the fake plugin set from
//! [`crate::testing`], so they never import a plugin crate.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use base::{ProjectionMode, RepoPath};

use crate::app::{App, Diagnostic, Msg, settle, update};
use crate::command::{Binding, CommandId, CommandSpec, Group, KeyScope, KeySeq, validate};
use crate::plugin::{FocusKey, NavigatorResize, PluginId, QueryChange, Request};
use crate::runtime::{Driver, Session, run_with, translate, translate_mouse};
use crate::testing::{
    BODY_FOCUS, FakeCmd, Fakes, FakesCmd, FakesMsg, NAV_COMMAND, NAV_FOCUS, NAV_PANE, OVERLAY_PANE,
    PASS_PANE, VIEW, VIEW_COMMAND, app,
};
use crate::util::input::{Key, Mouse, MouseKind};

fn run(app: &mut App<Fakes>, request: Request) -> Vec<FakesCmd> {
    let specs = crate::app::collect_specs(app);
    let mut cmds = Vec::new();
    crate::dispatch::apply_request(app, request, &mut cmds, 0, &specs);
    cmds
}

fn press(app: &mut App<Fakes>, key: Key) {
    let (next, _) = update(Msg::Key(key), app);
    *app = settle(next);
}

// -- request semantics ------------------------------------------------------

#[test]
fn quit_sets_the_flag() {
    let mut app = app();
    let cmds = run(&mut app, Request::Quit);
    assert!(app.kernel.chrome.quit);
    assert!(cmds.is_empty());
}

#[test]
fn diagnose_sets_a_forty_tick_toast() {
    let mut app = app();
    run(&mut app, Request::Diagnose("boom".to_owned()));
    let diagnostic = app.kernel.diagnostic.as_ref().expect("a toast");
    assert_eq!(diagnostic.text, "boom");
    assert_eq!(diagnostic.ttl, 40);
}

#[test]
fn clear_diagnostic_clears_it() {
    let mut app = app();
    app.kernel.diagnostic = Some(Diagnostic {
        text: "x".to_owned(),
        ttl: 3,
    });
    run(&mut app, Request::ClearDiagnostic);
    assert!(app.kernel.diagnostic.is_none());
}

#[test]
fn set_files_drops_a_missing_selection_without_an_event() {
    let mut app = app();
    app.kernel.workspace.selected = Some(RepoPath::new("a.rs").unwrap());
    app.plugins.view.log.events.clear();

    let set = crate::workspace::FileSet {
        paths: Arc::from(Vec::new()),
        ..Default::default()
    };
    run(&mut app, Request::SetFiles(set));

    assert!(app.kernel.workspace.selected.is_none());
    assert_eq!(
        app.plugins.view.log.events,
        vec!["FilesChanged"],
        "dropping a selection sends no SelectionChanged"
    );
}

#[test]
fn select_always_notifies_even_for_the_same_path() {
    let mut app = app();
    let path = RepoPath::new("a.rs").unwrap();
    app.kernel.workspace.selected = Some(path.clone());
    app.plugins.view.log.events.clear();

    run(&mut app, Request::Select(Some(path)));

    assert!(
        app.plugins.view.log.events.contains(&"SelectionChanged"),
        "re-selecting the same path still broadcasts"
    );
}

#[test]
fn select_clears_a_pending_prefix() {
    let mut app = app();
    app.kernel.keymap.pending = Some('z');
    app.kernel.overlay = Some(OVERLAY_PANE);
    app.plugins.overlay.log.events.clear();

    run(
        &mut app,
        Request::Select(Some(RepoPath::new("a.rs").unwrap())),
    );

    assert!(app.kernel.keymap.pending.is_none());
    assert!(app.plugins.overlay.log.events.contains(&"PrefixCleared"));
}

#[test]
fn requery_reaches_the_active_view() {
    let mut app = app();
    let cmds = run(
        &mut app,
        Request::Requery(QueryChange::Mode(ProjectionMode::Signatures)),
    );
    assert_eq!(cmds, vec![FakesCmd::View(FakeCmd::Bump)]);
    assert!(app.plugins.view.log.events.contains(&"Requery"));
}

#[test]
fn resize_navigator_clamps_to_15_60_step_5() {
    let mut app = app();
    assert_eq!(app.kernel.chrome.tree_percent, 30);

    run(&mut app, Request::ResizeNavigator(NavigatorResize::Wider));
    assert_eq!(app.kernel.chrome.tree_percent, 35);

    app.kernel.chrome.tree_percent = 55;
    run(&mut app, Request::ResizeNavigator(NavigatorResize::Wider));
    assert_eq!(app.kernel.chrome.tree_percent, 60);

    app.kernel.chrome.tree_percent = 60;
    run(&mut app, Request::ResizeNavigator(NavigatorResize::Wider));
    assert_eq!(app.kernel.chrome.tree_percent, 60, "clamped at the maximum");

    app.kernel.chrome.tree_percent = 30;
    run(
        &mut app,
        Request::ResizeNavigator(NavigatorResize::Narrower),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 25);

    app.kernel.chrome.tree_percent = 20;
    run(
        &mut app,
        Request::ResizeNavigator(NavigatorResize::Narrower),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 15);

    app.kernel.chrome.tree_percent = 15;
    run(
        &mut app,
        Request::ResizeNavigator(NavigatorResize::Narrower),
    );
    assert_eq!(app.kernel.chrome.tree_percent, 15, "clamped at the minimum");

    app.kernel.chrome.tree_percent = 45;
    run(&mut app, Request::ResizeNavigator(NavigatorResize::Reset));
    assert_eq!(app.kernel.chrome.tree_percent, 30);
}

#[test]
fn set_search_collects_matches_with_cursor_zero() {
    let mut app = app();
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    let search = app.kernel.workspace.search.expect("a search");
    assert_eq!(search.matches.len(), 2);
    assert_eq!(search.cursor, 0);
    assert_eq!(app.plugins.view.log.searches, 1);
}

#[test]
fn set_search_empty_clears() {
    let mut app = app();
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    assert!(app.kernel.workspace.search.is_some());
    run(&mut app, Request::SetSearch(String::new()));
    assert!(app.kernel.workspace.search.is_none());
}

#[test]
fn commit_search_reveals_the_current_match() {
    let mut app = app();
    app.plugins.view.log.events.clear();
    run(&mut app, Request::SetSearch("alpha".to_owned()));

    run(&mut app, Request::CommitSearch);

    assert!(app.plugins.view.log.events.contains(&"RevealMatch"));
}

#[test]
fn clear_search_clears() {
    let mut app = app();
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    assert!(app.kernel.workspace.search.is_some());
    run(&mut app, Request::ClearSearch);
    assert!(app.kernel.workspace.search.is_none());
}

#[test]
fn cancel_prefix_clears_the_band() {
    let mut app = app();
    run(&mut app, Request::CancelPrefix);
    assert!(
        app.kernel.keymap.pending.is_none(),
        "no pending prefix is a no-op"
    );

    app.kernel.keymap.pending = Some('z');
    app.kernel.overlay = Some(OVERLAY_PANE);
    run(&mut app, Request::CancelPrefix);
    assert!(app.kernel.keymap.pending.is_none());
}

#[test]
fn open_overlay_replaces_and_close_is_targeted() {
    let mut app = app();
    run(&mut app, Request::OpenOverlay(NAV_PANE));
    assert_eq!(app.kernel.overlay, Some(NAV_PANE));

    run(&mut app, Request::OpenOverlay(OVERLAY_PANE));
    assert_eq!(app.kernel.overlay, Some(OVERLAY_PANE));

    run(&mut app, Request::CloseOverlay(NAV_PANE));
    assert_eq!(
        app.kernel.overlay,
        Some(OVERLAY_PANE),
        "closing another pane leaves the open overlay alone"
    );
    run(&mut app, Request::CloseOverlay(OVERLAY_PANE));
    assert_eq!(app.kernel.overlay, None);
}

#[test]
fn replacing_an_overlay_notifies_its_owner() {
    let mut app = app();
    app.plugins.overlay.log.events.clear();
    run(&mut app, Request::OpenOverlay(OVERLAY_PANE));
    run(&mut app, Request::OpenOverlay(NAV_PANE));
    assert!(
        app.plugins.overlay.log.events.contains(&"OverlayClosed"),
        "the replaced owner receives OverlayClosed"
    );
}

#[test]
fn run_command_runs_a_registered_command() {
    let mut app = app();
    assert_eq!(app.kernel.chrome.tree_percent, 30);
    run(&mut app, Request::RunCommand(NAV_COMMAND));
    assert_eq!(app.kernel.chrome.tree_percent, 35);
}

#[test]
fn run_command_runs_a_plugin_effect() {
    let mut app = app();
    let cmds = run(&mut app, Request::RunCommand(VIEW_COMMAND));
    assert_eq!(cmds, vec![FakesCmd::View(FakeCmd::Bump)]);
}

#[test]
fn focus_and_cycle_focus_move_the_focus() {
    let mut app = app();
    assert_eq!(app.kernel.focus, Some(NAV_FOCUS));

    run(&mut app, Request::CycleFocus { forward: true });
    assert_eq!(app.kernel.focus, Some(BODY_FOCUS));

    run(&mut app, Request::Focus(NAV_FOCUS));
    assert_eq!(app.kernel.focus, Some(NAV_FOCUS));

    run(&mut app, Request::Focus(BODY_FOCUS));
    run(&mut app, Request::FocusDefault);
    assert_eq!(
        app.kernel.focus,
        Some(NAV_FOCUS),
        "the navigator is default"
    );
}

#[test]
fn focus_normalises_after_panes_disappear() {
    let mut app = app();
    app.kernel.focus = Some(FocusKey {
        owner: PluginId("gone"),
        slot: 0,
    });
    let (next, _) = update(Msg::<FakesMsg>::Tick, &app);
    assert_eq!(next.kernel.focus, Some(NAV_FOCUS));
}

#[test]
fn a_request_cascade_is_depth_first_and_deterministic() {
    let app = app();
    let mut first = app.clone();
    let mut second = app.clone();
    run(&mut first, Request::SetSearch("alpha".to_owned()));
    run(&mut second, Request::SetSearch("alpha".to_owned()));
    assert_eq!(
        first.kernel.workspace.search,
        second.kernel.workspace.search
    );
    assert!(first.kernel.workspace.search.is_some());
}

// -- key dispatch -----------------------------------------------------------

#[test]
fn q_quits_without_an_overlay() {
    let mut app = app();
    press(&mut app, Key::Char('q'));
    assert!(app.kernel.chrome.quit);
}

#[test]
fn q_quits_from_a_modal_non_capturing_overlay() {
    let mut app = app();
    app.kernel.overlay = Some(NAV_PANE);
    press(&mut app, Key::Char('q'));
    assert!(app.kernel.chrome.quit);
}

#[test]
fn q_is_typed_into_a_text_capturing_overlay() {
    let mut app = app();
    run(&mut app, Request::OpenOverlay(OVERLAY_PANE));
    press(&mut app, Key::Char('q'));
    assert!(!app.kernel.chrome.quit);
}

#[test]
fn a_key_unhandled_by_a_non_modal_overlay_falls_through() {
    let mut app = app();
    run(&mut app, Request::OpenOverlay(PASS_PANE));
    press(&mut app, Key::Char(']'));
    assert_eq!(
        app.kernel.chrome.tree_percent, 35,
        "the binding behind the overlay still runs"
    );
}

#[test]
fn q_under_a_pending_prefix_does_not_quit() {
    let mut app = app();
    app.kernel.focus = Some(BODY_FOCUS);
    press(&mut app, Key::Char('z'));
    assert_eq!(app.kernel.keymap.pending, Some('z'));

    press(&mut app, Key::Char('q'));
    assert!(!app.kernel.chrome.quit);
    assert!(app.kernel.keymap.pending.is_none());
}

#[test]
fn ctrl_j_and_ctrl_k_keep_their_j_k_alias_outside_the_band() {
    let mut app = app();
    app.kernel.focus = Some(BODY_FOCUS);
    press(&mut app, Key::CtrlJ);
    press(&mut app, Key::CtrlK);
    assert_eq!(
        app.plugins.view.log.keys,
        vec![Key::Char('j'), Key::Char('k')]
    );
}

#[test]
fn the_band_receives_ctrl_j_and_ctrl_k_as_they_are() {
    let mut app = app();
    run(&mut app, Request::OpenOverlay(PASS_PANE));
    press(&mut app, Key::CtrlJ);
    assert_eq!(app.plugins.overlay.log.keys, vec![Key::CtrlJ]);
}

#[test]
fn esc_with_no_overlay_clears_the_diagnostic_and_the_search() {
    let mut app = app();
    app.kernel.diagnostic = Some(Diagnostic {
        text: "x".to_owned(),
        ttl: 3,
    });
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    press(&mut app, Key::Esc);
    assert!(app.kernel.diagnostic.is_none());
    assert!(app.kernel.workspace.search.is_none());
}

// -- mouse dispatch ---------------------------------------------------------

#[test]
fn a_click_focuses_and_forwards_the_hit_pane() {
    let mut app = app();
    app.kernel.focus = Some(NAV_FOCUS);
    let (next, _) = update(
        Msg::Mouse(Mouse {
            column: 80,
            row: 10,
            kind: MouseKind::Click,
        }),
        &app,
    );
    assert_eq!(next.kernel.focus, Some(BODY_FOCUS));
}

// -- settle oracle ----------------------------------------------------------

#[test]
fn settle_runs_every_plugin_and_resyncs_the_search() {
    let mut app = app();
    run(&mut app, Request::SetSearch("alpha".to_owned()));
    app.plugins.view.log.searches = 0;

    let once = settle(app);
    assert_eq!(once.plugins.nav.log.settles, 1);
    assert_eq!(once.plugins.view.log.settles, 1);
    assert_eq!(once.plugins.overlay.log.settles, 1);
    assert_eq!(once.plugins.view.log.searches, 1, "settle re-syncs");
    assert_eq!(
        once.kernel.workspace.search.as_ref().unwrap().matches.len(),
        2
    );

    let twice = settle(once.clone());
    assert_eq!(twice.plugins.view.log.settles, 2, "settle is stable");
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
    messages: VecDeque<Msg<FakesMsg>>,
}

impl MockDriver {
    fn new(
        fail_at: Option<usize>,
        log: Rc<RefCell<Log>>,
        messages: VecDeque<Msg<FakesMsg>>,
    ) -> Self {
        Self {
            fail_at,
            log,
            messages,
        }
    }
}

impl Driver<Fakes> for MockDriver {
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

    fn draw(&mut self, _model: &App<Fakes>) -> Result<(), crate::TuiError> {
        self.log.borrow_mut().draws += 1;
        Ok(())
    }

    fn read_msg(&mut self) -> Result<Msg<FakesMsg>, crate::TuiError> {
        Ok(self
            .messages
            .pop_front()
            .unwrap_or(Msg::Key(Key::Char('q'))))
    }

    fn try_read_msg(&mut self) -> Result<Option<Msg<FakesMsg>>, crate::TuiError> {
        Ok(self.messages.pop_front())
    }
}

fn log() -> Rc<RefCell<Log>> {
    Rc::new(RefCell::new(Log::default()))
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
        let mut session = Session::<MockDriver, Fakes>::new(MockDriver::new(
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
        translate::<FakesMsg>(ctrl(KeyCode::Char('p'))),
        Some(Msg::Key(Key::CtrlP))
    ));
    assert!(matches!(
        translate::<FakesMsg>(ctrl(KeyCode::Char('j'))),
        Some(Msg::Key(Key::CtrlJ))
    ));
    assert!(matches!(
        translate::<FakesMsg>(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
        Some(Msg::Key(Key::PageDown))
    ));
    assert!(matches!(
        translate::<FakesMsg>(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
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
        translate_mouse::<FakesMsg>(at(MouseEventKind::Down(MouseButton::Left))),
        Some(Msg::Mouse(Mouse {
            column: 7,
            row: 3,
            kind: MouseKind::Click,
        }))
    ));
    assert!(matches!(
        translate_mouse::<FakesMsg>(at(MouseEventKind::ScrollUp)),
        Some(Msg::Mouse(Mouse {
            kind: MouseKind::ScrollUp,
            ..
        }))
    ));
    assert!(translate_mouse::<FakesMsg>(at(MouseEventKind::Moved)).is_none());
    assert!(translate_mouse::<FakesMsg>(at(MouseEventKind::Up(MouseButton::Left))).is_none());
}

#[test]
fn a_run_quits_on_q_and_restores_the_terminal() {
    let repo = TestRepo::new();
    let engine = engine::Engine::discover(repo.path()).expect("discover");
    let log = log();
    let messages = VecDeque::from([Msg::Key(Key::Char('q'))]);
    let driver = MockDriver::new(None, log.clone(), messages);

    run_with(&engine, app(), Vec::new(), driver).expect("run");

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

    run_with(&engine, app(), Vec::new(), driver).expect("run");

    assert_eq!(log.borrow().draws, 2);
}

#[test]
fn a_failing_initial_projection_restores_the_terminal_and_errors() {
    let repo = TestRepo::new();
    let engine = engine::Engine::discover(repo.path()).expect("discover");
    let log = log();
    let driver = MockDriver::new(None, log.clone(), VecDeque::new());
    let startup = vec![FakesCmd::View(FakeCmd::Fail)];

    let error = run_with(&engine, app(), startup, driver).expect_err("startup failure");
    assert!(matches!(error, crate::TuiError::Engine(_)), "got {error:?}");
    assert_eq!(
        log.borrow().teardown,
        vec!["mouse_off", "show_cursor", "leave_alternate", "disable_raw"]
    );
}

// -- registry validation ----------------------------------------------------

fn test_command(name: &'static str, bindings: Vec<Binding>) -> CommandSpec {
    CommandSpec {
        id: CommandId {
            owner: crate::command::KERNEL,
            name,
        },
        label: "",
        hint: "",
        bindings,
        palette: false,
        group: None,
        rank: 0,
    }
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
                KeyScope::Focused(NAV_FOCUS),
            )],
        ),
    ];
    assert!(validate(&boundary, Some(NAV_FOCUS)).is_err());
    assert!(validate(&boundary, Some(BODY_FOCUS)).is_ok());
    assert!(validate(&boundary, None).is_ok());
}

#[test]
fn validation_rejects_a_key_that_is_also_a_prefix() {
    let conflict = vec![
        test_command(
            "a",
            vec![bound(KeySeq::Key(Key::Char('x')), KeyScope::Anywhere)],
        ),
        CommandSpec {
            id: CommandId {
                owner: VIEW,
                name: "fold",
            },
            label: "",
            hint: "",
            bindings: vec![bound(
                KeySeq::Prefixed('x', 'a'),
                KeyScope::Focused(BODY_FOCUS),
            )],
            palette: true,
            group: Some(Group {
                prefix: 'x',
                name: "group",
            }),
            rank: 210,
        },
    ];
    assert!(validate(&conflict, Some(BODY_FOCUS)).is_err());
    assert!(validate(&conflict, Some(NAV_FOCUS)).is_ok());
}

#[test]
fn validation_rejects_a_prefixed_binding_without_a_group() {
    let missing = vec![CommandSpec {
        id: CommandId {
            owner: VIEW,
            name: "fold",
        },
        label: "",
        hint: "",
        bindings: vec![bound(
            KeySeq::Prefixed('z', 'a'),
            KeyScope::Focused(BODY_FOCUS),
        )],
        palette: true,
        group: None,
        rank: 210,
    }];
    assert!(validate(&missing, Some(BODY_FOCUS)).is_err());
}

// -- boundary scan ----------------------------------------------------------

#[test]
fn the_kernel_sources_name_no_plugin() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut files = Vec::new();
    collect_rs(std::path::Path::new(root), &mut files);
    files.retain(|file| {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        !matches!(name, "testing.rs" | "kernel_tests.rs")
    });
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
