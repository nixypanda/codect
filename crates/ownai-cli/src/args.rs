//! Command-line argument definitions (TECHNICAL_DESIGN.md section 14).
//!
//! `--mode` has no default: the product has not validated a preferred mode, so
//! the caller must choose explicitly.

use clap::{Parser, Subcommand, ValueEnum};

use ownai_core::ProjectionMode;

/// Repeated on every help surface so nobody trusts a focused diff without
/// knowing what it deliberately hides (section 14).
const FOCUSED_DIFF_HELP: &str = "Focused diffs are semantic: implementation-only changes (function bodies, \
     comments, whitespace) are invisible. Only changes that alter a projected \
     declaration appear.";

#[derive(Debug, Parser)]
#[command(
    name = "ownai",
    version,
    about = "Show and diff canonical Type and Signature projections of Git revisions",
    long_about = "OwnAI projects committed Elm, Haskell, Python, and Rust source files \
                  into canonical Type and Signature forms and diffs those projections \
                  between two revisions.\n\nFocused diffs are semantic: implementation-only \
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
    /// Nerd Font installed in the terminal; `OWNAI_ICONS=nerd` sets the same
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

        /// Revision to project (branch, tag, or object id); defaults to HEAD.
        #[arg(value_name = "REVISION", default_value = "HEAD")]
        revision: String,

        /// Limit the projection to these paths. Repeatable. A directory includes
        /// everything beneath it, and paths are relative to the current directory.
        #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
        paths: Vec<std::ffi::OsString>,

        /// Select a named area from `.ownai.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,
    },

    /// Show a focused projection diff between two revisions.
    #[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP)]
    Diff {
        /// Projection mode to compare.
        #[arg(long, value_enum)]
        mode: Mode,

        /// Base revision.
        #[arg(value_name = "BASE")]
        base: String,

        /// Target revision.
        #[arg(value_name = "TARGET")]
        target: String,

        /// Limit the projection to these paths. Repeatable. A directory includes
        /// everything beneath it, and paths are relative to the current directory.
        #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
        paths: Vec<std::ffi::OsString>,

        /// Select a named area from `.ownai.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,

        /// Annotate each changed declaration with semantic review judgments.
        #[arg(long, value_enum, requires = "decision_provider")]
        lens: Option<LensChoice>,

        /// Decision provider that answers the review questions. Required with
        /// `--lens`.
        #[arg(long, value_enum, value_name = "PROVIDER", requires = "lens")]
        decision_provider: Option<DecisionProviderChoice>,

        /// Override the model revision the provider reports for the review.
        #[arg(long, value_name = "MODEL", requires = "lens")]
        decision_model: Option<String>,

        /// Provider endpoint. Not supported by the `fake` provider.
        #[arg(long, value_name = "URL", requires = "lens")]
        decision_endpoint: Option<String>,
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

        /// Select a named area from `.ownai.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,
    },

    /// Browse a focused projection diff between two revisions.
    Diff {
        /// Projection mode to compare.
        #[arg(long, value_enum)]
        mode: Mode,

        /// Base revision.
        #[arg(value_name = "BASE")]
        base: String,

        /// Target revision.
        #[arg(value_name = "TARGET")]
        target: String,

        /// Limit the projection to these paths. Repeatable. A directory includes
        /// everything beneath it, and paths are relative to the current directory.
        #[arg(long = "path", short = 'p', value_name = "PATH", action = clap::ArgAction::Append)]
        paths: Vec<std::ffi::OsString>,

        /// Select a named area from `.ownai.toml`. Repeatable, and mutually
        /// exclusive with `--path`.
        #[arg(long = "area", short = 'a', value_name = "AREA", action = clap::ArgAction::Append, conflicts_with = "paths")]
        areas: Vec<String>,
    },
}

/// The two projection modes exposed on the command line.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Mode {
    /// Type declarations and aliases only.
    Types,
    /// Types plus function, method, value, constant, and static signatures.
    Signatures,
}

impl From<Mode> for ProjectionMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Types => ProjectionMode::Types,
            Mode::Signatures => ProjectionMode::Signatures,
        }
    }
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

/// The semantic lens selected by `--lens`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum LensChoice {
    /// Annotate each changed declaration with review judgments.
    Review,
}

/// The decision provider that answers semantic-lens questions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum DecisionProviderChoice {
    /// A deterministic, offline provider for tests and fixtures. It never opens
    /// a network connection and always returns the same answers for a request.
    Fake,
    /// The remote TypeSafe/Jev provider. Not yet available.
    Typesafe,
    /// The local Laya provider. Not yet available.
    Laya,
}

impl DecisionProviderChoice {
    /// The provider name attributed in a review document's header.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fake => "fake",
            Self::Typesafe => "typesafe",
            Self::Laya => "laya",
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("ownai").chain(args.iter().copied()))
    }

    #[test]
    fn lens_with_the_fake_provider_parses_for_diff() {
        let parsed = parse(&[
            "diff",
            "--mode",
            "signatures",
            "HEAD~1",
            "HEAD",
            "--lens",
            "review",
            "--decision-provider",
            "fake",
        ]);
        assert!(parsed.is_ok(), "{parsed:?}");
    }

    #[test]
    fn a_lens_without_a_provider_is_a_parse_error() {
        assert!(
            parse(&[
                "diff",
                "--mode",
                "signatures",
                "HEAD~1",
                "HEAD",
                "--lens",
                "review"
            ])
            .is_err()
        );
    }

    #[test]
    fn a_provider_without_a_lens_is_a_parse_error() {
        assert!(
            parse(&[
                "diff",
                "--mode",
                "signatures",
                "HEAD~1",
                "HEAD",
                "--decision-provider",
                "fake",
            ])
            .is_err()
        );
    }

    #[test]
    fn decision_model_and_endpoint_without_a_lens_are_parse_errors() {
        assert!(
            parse(&[
                "diff",
                "--mode",
                "signatures",
                "HEAD~1",
                "HEAD",
                "--decision-model",
                "custom",
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "diff",
                "--mode",
                "signatures",
                "HEAD~1",
                "HEAD",
                "--decision-endpoint",
                "http://localhost:1234",
            ])
            .is_err()
        );
    }

    #[test]
    fn lens_provider_options_parse_together() {
        let parsed = parse(&[
            "diff",
            "--mode",
            "signatures",
            "HEAD~1",
            "HEAD",
            "--lens",
            "review",
            "--decision-provider",
            "fake",
            "--decision-model",
            "custom",
        ]);
        assert!(parsed.is_ok(), "{parsed:?}");
    }

    #[test]
    fn plain_diff_and_show_invocations_still_parse() {
        assert!(parse(&["diff", "--mode", "types", "HEAD~1", "HEAD"]).is_ok());
        assert!(
            parse(&[
                "diff",
                "--mode",
                "signatures",
                "--path",
                "src",
                "HEAD~1",
                "HEAD"
            ])
            .is_ok()
        );
        assert!(parse(&["show", "--mode", "types"]).is_ok());
    }
}
