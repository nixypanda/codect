// Command-line argument definitions (TECHNICAL_DESIGN.md section 14).
//
// `--mode` has no default: the product has not validated a preferred mode, so
// the caller must choose explicitly.

use clap::{Parser, Subcommand, ValueEnum};

use base::ProjectionMode;

// Repeated on every help surface so nobody trusts a focused diff without
// knowing what it deliberately hides (section 14).
const FOCUSED_DIFF_HELP: &str = "Focused diffs are semantic: implementation-only changes (function bodies, \
     comments, whitespace) are invisible. Only changes that alter a projected \
     declaration appear.";

#[derive(Debug, Parser)]
#[command(
    name = "codect",
    version,
    about = "Show and diff canonical Type, Signature, and Test projections of Git revisions",
    long_about = "Codect projects committed Elm, Haskell, Python, and Rust source files \
                  into canonical Type, Signature, and Test forms and diffs those \
                  projections between two revisions.\n\nFocused diffs are semantic: implementation-only \
                  changes such as function bodies, comments, and whitespace are \
                  invisible. Only changes that alter a projected declaration appear.",
    after_help = FOCUSED_DIFF_HELP,
    after_long_help = FOCUSED_DIFF_HELP
)]
pub struct Cli {
    /// Control when ANSI color is emitted.
    #[arg(long, value_enum, default_value = "auto", global = true)]
    pub color: ColorChoice,

    /// Draw Nerd Font icons in the terminal frontend's file tree. Requires a
    /// Nerd Font installed in the terminal; `CODECT_ICONS=nerd` sets the same
    /// default.
    #[arg(long, value_enum, global = true)]
    pub icons: Option<IconChoice>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show a projection of one revision.
    #[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP)]
    Show {
        /// Projection mode to render.
        #[arg(long, value_enum)]
        mode: Mode,

        /// Output format. `text` is the canonical projection; `json` is the
        /// versioned `codect.show.v1` document with a mode-independent outline.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,

        /// Revision to project (branch, tag, or object id); defaults to HEAD.
        /// Mutually exclusive with `--stdin` and `--worktree`.
        #[arg(value_name = "REVISION")]
        revision: Option<String>,

        /// Limit the projection to these paths. Repeatable. A directory includes
        /// everything beneath it, and paths are relative to the current directory.
        #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
        paths: Vec<std::ffi::OsString>,

        /// Select a named area from `.codect.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,

        /// Read the source bytes from standard input instead of a revision.
        /// Requires exactly one `--path` to supply the language and repository
        /// context, and is mutually exclusive with `REVISION` and `--area`.
        #[arg(
            long,
            requires = "paths",
            conflicts_with_all = ["revision", "areas", "worktree"]
        )]
        stdin: bool,

        /// Read the source bytes from the file at `--path` on disk instead of a
        /// revision. Requires exactly one `--path`, and is mutually exclusive
        /// with `REVISION` and `--area`.
        #[arg(
            long,
            requires = "paths",
            conflicts_with_all = ["revision", "areas", "stdin"]
        )]
        worktree: bool,
    },

    /// Show a focused projection diff between two revisions.
    #[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP)]
    Diff {
        /// Projection mode to compare.
        #[arg(long, value_enum)]
        mode: Mode,

        /// Output format: text hunks or a versioned `codect.diff.v1` document.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,

        /// Base revision.
        #[arg(value_name = "BASE")]
        base: String,

        /// Target revision.
        #[arg(value_name = "TARGET")]
        target: String,

        /// Compare the merge base of BASE and TARGET against TARGET, so commits
        /// that only landed on BASE after the two branches diverged do not
        /// appear. Both sides must be commits.
        #[arg(long)]
        merge_base: bool,

        /// Limit the projection to these paths. Repeatable. A directory includes
        /// everything beneath it, and paths are relative to the current directory.
        #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
        paths: Vec<std::ffi::OsString>,

        /// Select a named area from `.codect.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,
    },

    /// Browse projections in the terminal.
    #[cfg(feature = "tui")]
    Tui {
        #[command(subcommand)]
        command: TuiCommand,
    },
}

/// Terminal frontend subcommands. Each form starts from an unambiguous state.
#[cfg(feature = "tui")]
#[derive(Debug, Subcommand)]
pub enum TuiCommand {
    /// Browse one projected revision.
    Show {
        /// Projection mode to render.
        #[arg(long, value_enum)]
        mode: Mode,

        /// Revision to project (branch, tag, or object id); defaults to HEAD.
        #[arg(value_name = "REVISION", default_value = "HEAD")]
        revision: String,

        /// Limit the projection to these paths. Repeatable. A directory includes
        /// everything beneath it, and paths are relative to the current directory.
        #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
        paths: Vec<std::ffi::OsString>,

        /// Select a named area from `.codect.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,
    },

    /// Browse a focused projection diff between two revisions.
    Diff {
        #[command(subcommand)]
        command: TuiDiffCommand,
    },
}

/// How a terminal diff traverses its revisions.
#[cfg(feature = "tui")]
#[derive(Debug, Subcommand)]
pub enum TuiDiffCommand {
    /// Compare the two revisions directly.
    Range {
        #[command(flatten)]
        args: TuiDiffArgs,
    },
    /// Browse first-parent commits between the revisions.
    Commits {
        #[command(flatten)]
        args: TuiDiffArgs,
    },
}

/// Arguments common to both terminal diff views.
#[cfg(feature = "tui")]
#[derive(Debug, clap::Args)]
pub struct TuiDiffArgs {
    /// Projection mode to compare.
    #[arg(long, value_enum)]
    pub mode: Mode,

    /// Base revision.
    #[arg(value_name = "BASE")]
    pub base: String,

    /// Target revision.
    #[arg(value_name = "TARGET")]
    pub target: String,

    /// Compare the merge base of BASE and TARGET against TARGET. Only the
    /// range view accepts this; the commits view walks first-parent history and
    /// has no merge base. Both sides must be commits.
    #[arg(long)]
    pub merge_base: bool,

    /// Limit the projection to these paths. Repeatable. A directory includes
    /// everything beneath it, and paths are relative to the current directory.
    #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
    pub paths: Vec<std::ffi::OsString>,

    /// Select a named area from `.codect.toml`. Repeatable, and mutually
    /// exclusive with `--path`.
    #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
    pub areas: Vec<String>,
}

/// The two projection modes exposed on the command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Mode {
    /// Type declarations and aliases only.
    Types,
    /// Types plus function, method, value, constant, and static signatures.
    Signatures,
    /// Signatures restricted to test declarations, in Rust and Python.
    Tests,
}

impl From<Mode> for ProjectionMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Types => ProjectionMode::Types,
            Mode::Signatures => ProjectionMode::Signatures,
            Mode::Tests => ProjectionMode::Tests,
        }
    }
}

/// The `show` and `diff` output format.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Format {
    /// The canonical text projection; byte-for-byte the historical output.
    Text,
    /// The versioned `codect.show.v1` JSON document.
    Json,
}

/// Color policy; `auto` defers terminal detection to the output layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ColorChoice {
    /// Emit ANSI color only when stdout is a terminal.
    Auto,
    /// Always emit ANSI color.
    Always,
    /// Never emit ANSI color.
    Never,
}

/// Whether the terminal frontend draws Nerd Font icons.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum IconChoice {
    /// No icons; readable on any font.
    None,
    /// Nerd Font folder and file-type glyphs.
    Nerd,
}
