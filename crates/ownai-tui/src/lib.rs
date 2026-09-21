//! The terminal frontend.
//!
//! This crate is the only layer that touches the terminal. It owns the run
//! loop, terminal lifecycle, and the interpretation of effects, and it follows
//! The Elm Architecture: `Model`, `Msg`, `Cmd`, `update`, and `view` are pure
//! and live in `app.rs` once the Show implementation lands.
//!
//! # Boundaries
//!
//! - No `clap` or `miette`. Argument parsing and diagnostic reporting belong to
//!   the command line.
//! - The crate never discovers a repository or reads `.ownai.toml`; it receives
//!   an [`Engine`] and a fully-built [`Selection`].
//!
//! The Show implementation — the TEA core, rendering, and the terminal driver
//! seam — is delivered on top of this scaffold.

use std::io::IsTerminal;

use ownai_core::ProjectionMode;
use ownai_engine::{Engine, EngineError, Selection};

/// Everything `run` needs beyond the engine, built by the caller.
///
/// The command line owns `argv` conversion and constructs the initial
/// selection, so the frontend never parses an argument or resolves an area.
#[derive(Clone, Debug)]
pub struct TuiOptions {
    /// The revision to project.
    pub revision: String,
    /// The projection mode to open with.
    pub mode: ProjectionMode,
    /// The initial normalized selection.
    pub selection: Selection,
    /// A short label for the initial scope, shown in the status bar.
    pub scope_label: String,
}

/// A failure that prevents the frontend from starting.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// Standard input or output is not a terminal, so no control sequence may
    /// be emitted.
    #[error("standard input and standard output must be terminals")]
    NotATerminal,

    /// The Show implementation has not been delivered yet. This remains until
    /// the TEA core and terminal runtime land.
    #[error("the terminal frontend is not implemented yet")]
    Unimplemented,

    /// The initial projection could not be produced. A terminal is restored
    /// before this reaches the caller.
    #[error(transparent)]
    Engine(#[from] EngineError),
}

/// Runs the terminal frontend until the user quits.
///
/// Verifies the terminal before emitting any control sequence; a non-terminal
/// invocation returns [`TuiError::NotATerminal`] without touching the screen.
pub fn run(_engine: Engine, _options: TuiOptions) -> Result<(), TuiError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal);
    }

    Err(TuiError::Unimplemented)
}
