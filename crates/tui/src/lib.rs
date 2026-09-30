//! The terminal frontend.
//!
//! This crate is the only layer that touches the terminal. It owns the run
//! loop, the terminal lifecycle, and the interpretation of effects, and it
//! follows The Elm Architecture: the pure `Model` (an [`app::App`]), `Msg`,
//! `Cmd`, `update`, and `view` live in [`app`] and [`view`], while this module
//! is the only imperative part.
//!
//! # Boundaries
//!
//! - No `clap` or `miette`. Argument parsing and diagnostic reporting belong to
//!   the command line.
//! - The crate never discovers a repository or reads `.ownai.toml`; it receives
//!   an [`Engine`] and a fully-built [`Selection`].
//! - Engine calls are effects: they are described by a [`app::Cmd`] and only
//!   ever run here, never inside `update` or `view`.

use std::io::{IsTerminal, Stdout};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use crossterm::cursor;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind, MouseEvent};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use engine::{Engine, EngineError};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

mod action;
mod api;
mod app;
mod component;
mod page;
mod render;
mod route;
mod util;
mod view;

#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench;

use app::{App, Cmd, DiffRequest, Msg, ShowRequest, coalesce_commit_loads, settle, update};
pub use app::{DiffView, LoadRequest};
pub use render::icons::IconStyle;
use render::icons::Icons;
use render::theme::Theme;
use util::input::{Key, Mouse, MouseKind};
use view::view;

// How long the driver waits for input before emitting a tick, which drives the
// spinner and lets a diagnostic expire without another keypress.
const TICK_INTERVAL: Duration = Duration::from_millis(150);

const MAX_INPUT_BATCH: usize = 256;

// The longest a batch may spend draining. Bounds how long a continuous stream
// of events can delay drawing.
const INPUT_BATCH_BUDGET: Duration = Duration::from_millis(8);

/// Everything `run` needs beyond the engine, built by the caller.
///
/// The command line owns `argv` conversion and constructs the initial
/// selection, so the frontend never parses an argument or resolves an area.
#[derive(Clone, Debug)]
pub struct TuiOptions {
    /// The projection to open with; `m` re-runs it in the other mode.
    pub request: LoadRequest,
    /// A short label for the initial scope, shown in the status bar.
    pub scope_label: String,
    /// Whether the file tree draws Nerd Font icons.
    pub icons: IconStyle,
}

/// A failure that prevents the frontend from starting or continuing.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// Standard input or output is not a terminal, so no control sequence may
    /// be emitted.
    #[error("standard input and standard output must be terminals")]
    NotATerminal,

    /// The terminal could not be set up, read, or drawn.
    #[error("the terminal could not be used")]
    Terminal(#[source] std::io::Error),

    /// The initial projection could not be produced. The terminal is restored
    /// before this reaches the caller.
    #[error(transparent)]
    Engine(#[from] EngineError),
}

/// Runs the terminal frontend until the user quits.
///
/// Verifies the terminal before emitting any control sequence; a non-terminal
/// invocation returns [`TuiError::NotATerminal`] without touching the screen.
pub fn run(engine: Engine, options: TuiOptions) -> Result<(), TuiError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal);
    }
    install_panic_hook();
    // Query the terminal's background before the event reader is first polled:
    // `terminal-colorsaurus` reads the reply from standard input itself, and the
    // query briefly consumes any keystroke typed during startup.
    let theme = Theme::detect();
    run_with(engine, options, theme, CrosstermDriver::default())
}

// The imperative seam: terminal lifecycle, event reading, and effect
// interpretation. Generic over the driver so tests can inject one.
//
// The theme is resolved by the caller so tests never touch the terminal.
fn run_with<D: Driver>(
    engine: Engine,
    options: TuiOptions,
    theme: Theme,
    driver: D,
) -> Result<(), TuiError> {
    let mut session = Session::new(driver);
    session.setup()?;

    let (width, height) = session.driver().size()?;
    let request = options.request.clone();
    let mut model = App::new(
        engine.root().display().to_string(),
        request.clone(),
        options.scope_label,
        width,
        height,
        theme,
        Icons::new(options.icons),
    );

    // The initial projection is the startup effect. Unlike a later reload, a
    // failure here is fatal: there is no previous screen to keep.
    // Draw the busy state first so the spinner is visible while it runs.
    session.driver().draw(&model)?;
    let startup = interpret(&engine, request_into_cmd(request));
    match startup {
        Msg::ShowLoaded {
            result: Err(error), ..
        }
        | Msg::DiffLoaded {
            result: Err(error), ..
        }
        | Msg::HistoryLoaded {
            result: Err(error), ..
        } => return Err(TuiError::Engine(*error)),
        msg => {
            model = update(msg, &model).0;
        }
    }

    // The frame is redrawn only when something changed or is animating.
    let mut dirty = true;
    loop {
        if dirty {
            // Selection-dependent work runs once per frame, on the final state.
            model = settle(model);
            session.driver().draw(&model)?;
        }
        let msg = session.driver().read_msg()?;
        // An idle tick has nothing to animate, so it neither updates nor redraws.
        if matches!(msg, Msg::Tick) && !model.is_busy() && model.diagnostic.is_none() {
            dirty = false;
            continue;
        }

        // Fold every event already queued into one batch, so a burst of tree
        // navigation costs one settle and one frame instead of one per row.
        let mut batch = vec![msg];
        let deadline = Instant::now() + INPUT_BATCH_BUDGET;
        while batch.len() < MAX_INPUT_BATCH && Instant::now() < deadline {
            match session.driver().try_read_msg()? {
                Some(msg) => batch.push(msg),
                None => break,
            }
        }

        let mut next = model;
        let mut cmds = Vec::new();
        for msg in batch {
            let (updated, produced) = update(msg, &next);
            next = updated;
            cmds.extend(produced);
        }
        coalesce_commit_loads(&mut next, &mut cmds);
        next = settle(next);
        if !cmds.is_empty() {
            // Draw the busy state before a blocking effect runs.
            session.driver().draw(&next)?;
        }
        let mut index = 0;
        while index < cmds.len() {
            let cmd = cmds[index].clone();
            let completed = interpret(&engine, cmd);
            let (updated, produced) = update(completed, &next);
            next = updated;
            cmds.extend(produced);
            index += 1;
        }
        model = next;
        dirty = true;
        if model.chrome.quit {
            break;
        }
    }

    // `Session`'s drop restores the terminal, on this path and on any error.
    Ok(())
}

fn request_into_cmd(request: LoadRequest) -> Cmd {
    match request {
        LoadRequest::Show {
            revision,
            mode,
            selection,
        } => Cmd::Show(ShowRequest {
            revision,
            mode,
            selection,
        }),
        LoadRequest::Diff {
            base,
            target,
            mode,
            selection,
            view,
        } => Cmd::Diff(DiffRequest {
            base,
            target,
            mode,
            selection,
            view,
        }),
    }
}

fn interpret(engine: &Engine, cmd: Cmd) -> Msg {
    match cmd {
        Cmd::Show(request) => {
            let result = engine
                .show(&request.revision, request.mode, &request.selection)
                .map(Arc::from)
                .map_err(Box::new);
            Msg::ShowLoaded { request, result }
        }
        Cmd::Diff(request) => match request.view {
            DiffView::Range => {
                let result = engine
                    .diff(
                        &request.base,
                        &request.target,
                        request.mode,
                        &request.selection,
                    )
                    .map(Arc::from)
                    .map_err(Box::new);
                Msg::DiffLoaded { request, result }
            }
            DiffView::Commits => {
                let result = engine
                    .first_parent_steps(&request.base, &request.target)
                    .and_then(|steps| {
                        let diffs: Vec<_> = match steps.first() {
                            Some(step) => engine.diff(
                                &step.parent_id.to_string(),
                                &step.commit_id.to_string(),
                                request.mode,
                                &request.selection,
                            )?,
                            None => Vec::new(),
                        };
                        Ok((Arc::from(steps), Arc::from(diffs)))
                    })
                    .map_err(Box::new);
                Msg::HistoryLoaded { request, result }
            }
        },
        Cmd::Step {
            request,
            index,
            step,
        } => {
            let result = engine
                .diff(
                    &step.parent_id.to_string(),
                    &step.commit_id.to_string(),
                    request.mode,
                    &request.selection,
                )
                .map(Arc::from)
                .map_err(Box::new);
            Msg::StepLoaded { index, result }
        }
        Cmd::LoadAreas => Msg::AreasLoaded(engine.load_areas().map_err(Box::new)),
    }
}

trait Driver {
    // Stages terminal setup. On failure, the steps that succeeded are undone
    // by [`Driver::teardown`].
    fn setup(&mut self) -> Result<(), TuiError>;
    // Undoes exactly the steps that succeeded, in reverse order. Best-effort.
    fn teardown(&mut self);
    fn size(&mut self) -> Result<(u16, u16), TuiError>;
    fn draw(&mut self, model: &App) -> Result<(), TuiError>;
    fn read_msg(&mut self) -> Result<Msg, TuiError>;
    // A message for an event that is already available, or `None` when the
    // input queue is empty. Never blocks.
    fn try_read_msg(&mut self) -> Result<Option<Msg>, TuiError>;
}

// Owns a driver and guarantees its teardown on drop, whether the runtime
// returns normally, returns an error, or unwinds.
struct Session<D: Driver> {
    driver: D,
    attempted: bool,
}

impl<D: Driver> Session<D> {
    fn new(driver: D) -> Self {
        Self {
            driver,
            attempted: false,
        }
    }

    fn setup(&mut self) -> Result<(), TuiError> {
        // Marked before the attempt so a partial setup is still cleaned up.
        self.attempted = true;
        self.driver.setup()
    }

    fn driver(&mut self) -> &mut D {
        &mut self.driver
    }
}

impl<D: Driver> Drop for Session<D> {
    fn drop(&mut self) {
        if self.attempted {
            self.driver.teardown();
        }
    }
}

// Toggles mouse capture, reporting only button and wheel events with SGR
// coordinates.
//
// crossterm's stock `EnableMouseCapture` also switches on all-motion tracking
// (`?1003h`), which floods the event loop with `Moved` events. Enabling only
// normal tracking (`?1000h`) plus SGR coordinates keeps the queue quiet.
struct MouseCaptured(bool);

impl crossterm::Command for MouseCaptured {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        if self.0 {
            f.write_str("\x1b[?1000h\x1b[?1006h")
        } else {
            f.write_str("\x1b[?1006l\x1b[?1000l")
        }
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        use crossterm::Command;
        if self.0 {
            crossterm::event::EnableMouseCapture.execute_winapi()
        } else {
            crossterm::event::DisableMouseCapture.execute_winapi()
        }
    }
}

#[derive(Default)]
struct CrosstermDriver {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
    raw: bool,
    alternate: bool,
    hidden: bool,
    mouse: bool,
}

impl Driver for CrosstermDriver {
    fn setup(&mut self) -> Result<(), TuiError> {
        enable_raw_mode().map_err(TuiError::Terminal)?;
        self.raw = true;

        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen).map_err(TuiError::Terminal)?;
        self.alternate = true;

        execute!(stdout, cursor::Hide).map_err(TuiError::Terminal)?;
        self.hidden = true;

        execute!(stdout, MouseCaptured(true)).map_err(TuiError::Terminal)?;
        self.mouse = true;

        let backend = CrosstermBackend::new(std::io::stdout());
        self.terminal = Some(Terminal::new(backend).map_err(TuiError::Terminal)?);
        Ok(())
    }

    fn teardown(&mut self) {
        self.terminal = None;
        let mut stdout = std::io::stdout();
        if self.mouse {
            let _ = execute!(stdout, MouseCaptured(false));
            self.mouse = false;
        }
        if self.hidden {
            let _ = execute!(stdout, cursor::Show);
            self.hidden = false;
        }
        if self.alternate {
            let _ = execute!(stdout, LeaveAlternateScreen);
            self.alternate = false;
        }
        if self.raw {
            let _ = disable_raw_mode();
            self.raw = false;
        }
    }

    fn size(&mut self) -> Result<(u16, u16), TuiError> {
        crossterm::terminal::size().map_err(TuiError::Terminal)
    }

    fn draw(&mut self, model: &App) -> Result<(), TuiError> {
        let terminal = self.terminal.as_mut().ok_or_else(not_a_terminal)?;
        terminal
            .draw(|frame| view(model, frame))
            .map(|_| ())
            .map_err(TuiError::Terminal)
    }

    fn read_msg(&mut self) -> Result<Msg, TuiError> {
        loop {
            // Wait for an event, or emit a tick so the view can animate.
            if !event::poll(TICK_INTERVAL).map_err(TuiError::Terminal)? {
                return Ok(Msg::Tick);
            }
            match event::read().map_err(TuiError::Terminal)? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if let Some(msg) = translate(key) {
                        return Ok(msg);
                    }
                }
                Event::Mouse(mouse) => {
                    if let Some(msg) = translate_mouse(mouse) {
                        return Ok(msg);
                    }
                }
                Event::Resize(width, height) => return Ok(Msg::Resize { width, height }),
                _ => {}
            }
        }
    }

    fn try_read_msg(&mut self) -> Result<Option<Msg>, TuiError> {
        loop {
            if !event::poll(Duration::ZERO).map_err(TuiError::Terminal)? {
                return Ok(None);
            }
            match event::read().map_err(TuiError::Terminal)? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if let Some(msg) = translate(key) {
                        return Ok(Some(msg));
                    }
                }
                Event::Mouse(mouse) => {
                    if let Some(msg) = translate_mouse(mouse) {
                        return Ok(Some(msg));
                    }
                }
                Event::Resize(width, height) => return Ok(Some(Msg::Resize { width, height })),
                _ => {}
            }
        }
    }
}

fn not_a_terminal() -> TuiError {
    TuiError::Terminal(std::io::Error::other("the terminal is not set up"))
}

fn translate(key: KeyEvent) -> Option<Msg> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let key = match key.code {
        KeyCode::Char('c') if control => Key::CtrlC,
        KeyCode::Char('d') if control => Key::CtrlD,
        KeyCode::Char('f') if control => Key::CtrlF,
        KeyCode::Char('p') if control => Key::CtrlP,
        KeyCode::Char('u') if control => Key::CtrlU,
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        _ => return None,
    };
    Some(Msg::Key(key))
}

// Translates a terminal mouse event, dropping motion, drag, release, and
// non-left buttons so they never reach the core.
fn translate_mouse(mouse: MouseEvent) -> Option<Msg> {
    use crossterm::event::{MouseButton, MouseEventKind};

    let kind = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => MouseKind::Click,
        MouseEventKind::ScrollUp => MouseKind::ScrollUp,
        MouseEventKind::ScrollDown => MouseKind::ScrollDown,
        _ => return None,
    };
    Some(Msg::Mouse(Mouse {
        column: mouse.column,
        row: mouse.row,
        kind,
    }))
}

// Installs a panic hook that restores the terminal before the panic message
// is printed, so a crash does not leave the alternate screen active.
fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

// Best-effort terminal restoration that never panics.
fn restore_terminal() {
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, MouseCaptured(false));
    let _ = execute!(stdout, cursor::Show);
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use base::{ProjectionMode, Selection};

    use super::*;

    #[derive(Default)]
    struct Log {
        setup: Vec<&'static str>,
        teardown: Vec<&'static str>,
        draws: usize,
    }

    struct MockDriver {
        fail_at: Option<usize>,
        log: Rc<RefCell<Log>>,
        messages: VecDeque<Msg>,
    }

    impl MockDriver {
        fn new(fail_at: Option<usize>, log: Rc<RefCell<Log>>, messages: VecDeque<Msg>) -> Self {
            Self {
                fail_at,
                log,
                messages,
            }
        }
    }

    impl Driver for MockDriver {
        fn setup(&mut self) -> Result<(), TuiError> {
            for (index, step) in ["raw", "alternate", "hidden", "mouse"]
                .into_iter()
                .enumerate()
            {
                if self.fail_at == Some(index) {
                    return Err(TuiError::Terminal(std::io::Error::other("setup failed")));
                }
                self.log.borrow_mut().setup.push(step);
            }
            Ok(())
        }

        fn teardown(&mut self) {
            let mut log = self.log.borrow_mut();
            // Undo in reverse order, matching the real driver.
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

        fn size(&mut self) -> Result<(u16, u16), TuiError> {
            Ok((100, 30))
        }

        fn draw(&mut self, _model: &App) -> Result<(), TuiError> {
            self.log.borrow_mut().draws += 1;
            Ok(())
        }

        fn read_msg(&mut self) -> Result<Msg, TuiError> {
            Ok(self
                .messages
                .pop_front()
                .unwrap_or(Msg::Key(Key::Char('q'))))
        }

        fn try_read_msg(&mut self) -> Result<Option<Msg>, TuiError> {
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
            let mut session = Session::new(MockDriver::new(fail_at, log.clone(), VecDeque::new()));
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
            translate(ctrl(KeyCode::Char('p'))),
            Some(Msg::Key(Key::CtrlP))
        ));
        assert!(matches!(
            translate(ctrl(KeyCode::Char('f'))),
            Some(Msg::Key(Key::CtrlF))
        ));
        assert!(matches!(
            translate(ctrl(KeyCode::Char('d'))),
            Some(Msg::Key(Key::CtrlD))
        ));
        assert!(matches!(
            translate(ctrl(KeyCode::Char('u'))),
            Some(Msg::Key(Key::CtrlU))
        ));
        assert!(matches!(
            translate(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            Some(Msg::Key(Key::PageDown))
        ));
        assert!(matches!(
            translate(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
            Some(Msg::Key(Key::Char('p')))
        ));
    }

    #[test]
    fn mouse_clicks_and_wheel_translate_but_motion_does_not() {
        use crossterm::event::{MouseButton, MouseEventKind};

        let at = |kind| MouseEvent {
            kind,
            column: 7,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };

        assert!(matches!(
            translate_mouse(at(MouseEventKind::Down(MouseButton::Left))),
            Some(Msg::Mouse(Mouse {
                column: 7,
                row: 3,
                kind: MouseKind::Click,
            }))
        ));
        assert!(matches!(
            translate_mouse(at(MouseEventKind::ScrollUp)),
            Some(Msg::Mouse(Mouse {
                kind: MouseKind::ScrollUp,
                ..
            }))
        ));
        assert!(matches!(
            translate_mouse(at(MouseEventKind::ScrollDown)),
            Some(Msg::Mouse(Mouse {
                kind: MouseKind::ScrollDown,
                ..
            }))
        ));
        assert!(translate_mouse(at(MouseEventKind::Moved)).is_none());
        assert!(translate_mouse(at(MouseEventKind::Up(MouseButton::Left))).is_none());
        assert!(translate_mouse(at(MouseEventKind::Down(MouseButton::Right))).is_none());
    }

    #[test]
    fn a_run_quits_on_q_and_restores_the_terminal() {
        let repo = TestRepo::new();
        let engine = Engine::discover(repo.path()).expect("discover");
        let log = log();
        let messages = VecDeque::from([Msg::Key(Key::Char('q'))]);
        let driver = MockDriver::new(None, log.clone(), messages);

        let options = TuiOptions {
            request: LoadRequest::Show {
                revision: "HEAD".to_owned(),
                mode: ProjectionMode::Types,
                selection: Selection::all(),
            },
            scope_label: "all".to_owned(),
            icons: IconStyle::None,
        };
        run_with(engine, options, Theme::dark(), driver).expect("run");

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
        let engine = Engine::discover(repo.path()).expect("discover");
        let log = log();
        let messages = VecDeque::from([
            Msg::Key(Key::Down),
            Msg::Key(Key::Down),
            Msg::Key(Key::Down),
            Msg::Key(Key::Char('q')),
        ]);
        let driver = MockDriver::new(None, log.clone(), messages);

        let options = TuiOptions {
            request: LoadRequest::Show {
                revision: "HEAD".to_owned(),
                mode: ProjectionMode::Types,
                selection: Selection::all(),
            },
            scope_label: "all".to_owned(),
            icons: IconStyle::None,
        };
        run_with(engine, options, Theme::dark(), driver).expect("run");

        // One frame for the initial busy state and one for the whole burst. The
        // old one-message-per-frame loop would have drawn once per event.
        assert_eq!(log.borrow().draws, 2);
    }

    #[test]
    fn a_failing_initial_projection_restores_the_terminal_and_errors() {
        let repo = TestRepo::new();
        let engine = Engine::discover(repo.path()).expect("discover");
        let log = log();
        let driver = MockDriver::new(None, log.clone(), VecDeque::new());

        let options = TuiOptions {
            // There is no such revision in an empty repository.
            request: LoadRequest::Show {
                revision: "no-such-revision".to_owned(),
                mode: ProjectionMode::Types,
                selection: Selection::all(),
            },
            scope_label: "all".to_owned(),
            icons: IconStyle::None,
        };
        let error = run_with(engine, options, Theme::dark(), driver).expect_err("startup failure");
        assert!(matches!(error, TuiError::Engine(_)), "got {error:?}");
        assert_eq!(
            log.borrow().teardown,
            vec!["mouse_off", "show_cursor", "leave_alternate", "disable_raw"]
        );
    }

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
                "user.name=OwnAI Test",
                "-c",
                "user.email=ownai@example.invalid",
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
}
