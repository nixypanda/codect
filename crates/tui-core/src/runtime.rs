//! The terminal runtime: the event loop, the terminal lifecycle, and the panic
//! hook. This is the only imperative part of the kernel.
//!
//! The loop is generic over a [`PluginSet`]: effects run through
//! `S::perform`, results fold back as `Msg::Plugin`, and `after_batch` runs
//! once per input burst before settle. `tui` and `tui-diff` use [`run`] and
//! [`TuiError`]; `tui` re-exports both.

use std::io::{Stdout, Write};
use std::sync::Once;
use std::time::{Duration, Instant};

use crossterm::cursor;
use crossterm::event::{self, Event, KeyEvent, KeyEventKind, MouseEvent};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use engine::Engine;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::{App, Msg, collect_specs, cx_for, settle, update};
use crate::dispatch::process_wrapped;
use crate::plugin::{PluginSet, Size};
use crate::util::input::{Key, Mouse, MouseKind};

/// The error a session can fail with.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("standard input and standard output must be terminals")]
    NotATerminal,

    #[error("the terminal could not be used")]
    Terminal(#[source] std::io::Error),

    #[error(transparent)]
    Engine(#[from] engine::EngineError),
}

const TICK_INTERVAL: Duration = Duration::from_millis(150);

const MAX_INPUT_BATCH: usize = 256;

const INPUT_BATCH_BUDGET: Duration = Duration::from_millis(8);

/// The driver abstraction the runtime draws and reads through.
pub trait Driver<S: PluginSet> {
    fn setup(&mut self) -> Result<(), TuiError>;
    fn teardown(&mut self);
    fn size(&mut self) -> Result<(u16, u16), TuiError>;
    fn draw(&mut self, model: &App<S>) -> Result<(), TuiError>;
    fn read_msg(&mut self) -> Result<Msg<S::Msg>, TuiError>;
    fn try_read_msg(&mut self) -> Result<Option<Msg<S::Msg>>, TuiError>;
}

pub struct Session<D: Driver<S>, S: PluginSet> {
    driver: D,
    attempted: bool,
    _marker: std::marker::PhantomData<fn() -> S>,
}

impl<D: Driver<S>, S: PluginSet> Session<D, S> {
    pub fn new(driver: D) -> Self {
        Self {
            driver,
            attempted: false,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn setup(&mut self) -> Result<(), TuiError> {
        self.attempted = true;
        self.driver.setup()
    }

    pub fn driver(&mut self) -> &mut D {
        &mut self.driver
    }
}

impl<D: Driver<S>, S: PluginSet> Drop for Session<D, S> {
    fn drop(&mut self) {
        if self.attempted {
            self.driver.teardown();
        }
    }
}

/// Run the event loop over an already-built `App`.
pub fn run<S: PluginSet>(
    engine: &Engine,
    app: App<S>,
    startup: Vec<S::Cmd>,
) -> Result<(), TuiError> {
    run_with(engine, app, startup, CrosstermDriver::default())
}

pub fn run_with<S: PluginSet, D: Driver<S>>(
    engine: &Engine,
    mut app: App<S>,
    startup: Vec<S::Cmd>,
    driver: D,
) -> Result<(), TuiError> {
    let mut session = Session::new(driver);
    session.setup()?;
    let (width, height) = session.driver().size()?;
    app.kernel.size = Size { width, height };

    session.driver().draw(&app)?;
    for cmd in startup {
        let msg = S::perform(engine, cmd);
        match S::startup_failure(msg) {
            Err(error) => return Err(TuiError::Engine(*error)),
            Ok(msg) => {
                app = update(Msg::Plugin(msg), &app).0;
            }
        }
    }

    let mut dirty = true;
    loop {
        if dirty {
            app = settle(app);
            session.driver().draw(&app)?;
        }
        let msg = session.driver().read_msg()?;
        if matches!(msg, Msg::Tick) && !app.plugins.busy() && app.kernel.diagnostic.is_none() {
            dirty = false;
            continue;
        }

        let mut batch = vec![msg];
        let deadline = Instant::now() + INPUT_BATCH_BUDGET;
        while batch.len() < MAX_INPUT_BATCH && Instant::now() < deadline {
            match session.driver().try_read_msg()? {
                Some(msg) => batch.push(msg),
                None => break,
            }
        }

        let mut next = app;
        let mut cmds = Vec::new();
        for msg in batch {
            let (updated, produced) = update(msg, &next);
            next = updated;
            cmds.extend(produced);
        }
        let specs = collect_specs(&next);
        let outs = {
            let App { kernel, plugins } = &mut next;
            let overlay = kernel.overlay;
            let busy = plugins.busy();
            let cx = cx_for(kernel, overlay, busy, &specs);
            plugins.after_batch(&cmds, &cx)
        };
        process_wrapped(&mut next, outs, &mut cmds, 0, &specs);
        next = settle(next);
        if !cmds.is_empty() {
            session.driver().draw(&next)?;
        }
        let mut index = 0;
        while index < cmds.len() {
            let cmd = cmds[index].clone();
            let completed = S::perform(engine, cmd);
            let (updated, produced) = update(Msg::Plugin(completed), &next);
            next = updated;
            cmds.extend(produced);
            index += 1;
        }
        app = next;
        dirty = true;
        if app.kernel.chrome.quit {
            break;
        }
    }

    Ok(())
}

// Toggles mouse capture, reporting only button and wheel events with SGR
// coordinates.
//
// crossterm's stock `EnableMouseCapture` also switches on all-motion tracking
// (`?1003h`), which floods the event loop with `Moved` events. Enabling only
// normal tracking (`?1000h`) plus SGR coordinates keeps the queue quiet.
pub struct MouseCaptured(bool);

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
pub struct CrosstermDriver {
    terminal: Option<Terminal<CrosstermBackend<Stdout>>>,
    raw: bool,
    alternate: bool,
    hidden: bool,
    mouse: bool,
}

impl<S: PluginSet> Driver<S> for CrosstermDriver {
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

    fn draw(&mut self, model: &App<S>) -> Result<(), TuiError> {
        let terminal = self.terminal.as_mut().ok_or_else(not_a_terminal)?;
        terminal
            .draw(|frame| crate::view::view(model, frame))
            .map(|_| ())
            .map_err(TuiError::Terminal)
    }

    fn read_msg(&mut self) -> Result<Msg<S::Msg>, TuiError> {
        loop {
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

    fn try_read_msg(&mut self) -> Result<Option<Msg<S::Msg>>, TuiError> {
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

pub fn translate<M>(key: KeyEvent) -> Option<Msg<M>> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let key = match key.code {
        KeyCode::Char('c') if control => Key::CtrlC,
        KeyCode::Char('d') if control => Key::CtrlD,
        KeyCode::Char('f') if control => Key::CtrlF,
        KeyCode::Char('j') if control => Key::CtrlJ,
        KeyCode::Char('k') if control => Key::CtrlK,
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

pub fn translate_mouse<M>(mouse: MouseEvent) -> Option<Msg<M>> {
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

pub fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

fn restore_terminal() {
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, MouseCaptured(false));
    let _ = execute!(stdout, cursor::Show);
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = disable_raw_mode();
    let _ = stdout.flush();
}
