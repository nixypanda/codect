//! The `ownai` command-line interface.
//!
//! Thin entry point: argument parsing and exit-code mapping only. The
//! Git-aware pipeline lives in `command.rs`, and core stays Git-free
//! (TECHNICAL_DESIGN.md sections 3, 14).

mod args;
mod command;
mod decision;
mod output;
mod pathspec;
mod review;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    // `clap` prints usage errors to stderr and exits 2; `--help` and
    // `--version` exit 0 (section 14). Parsing happens before any work, so a
    // malformed invocation never opens a repository or reads objects.
    let cli = args::Cli::parse();

    match command::run(&cli) {
        Ok(command::CommandOutcome::Success) => ExitCode::SUCCESS,
        // The complete review document is already on stdout; a failed unit is
        // signaled by the exit code with no extra diagnostic.
        Ok(command::CommandOutcome::ReviewFailed) => ExitCode::FAILURE,
        // Repository, revision, object, UTF-8, and parse failures all share
        // exit 1 (section 14); the report goes to stderr so stdout stays a
        // valid, complete document or empty.
        Err(error) => {
            eprint!("{}", output::render_diagnostic(&error));
            ExitCode::FAILURE
        }
    }
}
