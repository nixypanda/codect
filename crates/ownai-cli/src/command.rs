//! The thin command layer.
//!
//! Everything Git-aware lives in `ownai-engine`; this module only builds a
//! selection from `argv`, invokes the engine, and renders the result or a
//! diagnostic. Keeping the shared pipeline in the engine is what lets the
//! terminal frontend reuse it without duplicating behavior.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::io::IsTerminal as _;
use std::io::Write as _;
use std::path::Path;

use ownai_core::{
    AreaSet, DiagnosticContext, PathSelection, PathSelectionError, ProjectedFile, ProjectionError,
    SourceSpan, diff_document, show_document,
};
use ownai_engine::config::ConfigError;
use ownai_engine::{
    Engine, EngineError, FileDiff, ReviewError, ReviewLens, ReviewOutcome, Selection,
    SelectionGroup,
};
use ownai_git::GitError;

#[cfg(feature = "tui")]
use crate::args::IconChoice;
#[cfg(feature = "tui")]
use crate::args::TuiCommand;
use crate::args::{Cli, ColorChoice, Command as CliCommand, DecisionProviderChoice, LensChoice};
use crate::decision::{self, ProviderBuildError, ProviderRoute};
use crate::disclosure::{self, DisclosureError, EndpointError};
use crate::output::{self, DocumentKind};
use crate::pathspec::{self, PathArgError};
use crate::review::{self, ReviewRenderOptions};

/// The default serialized-state byte limit for one review unit.
pub const DEFAULT_MAX_STATE_BYTES: usize = 64 * 1024;

/// The successful result of a command.
///
/// A review document is written before the exit status is chosen, so a failed
/// unit is signaled only by [`CommandOutcome::ReviewFailed`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    /// The command completed; a review, if any, had no failed unit.
    Success,
    /// The review document was written, but at least one unit failed.
    ReviewFailed,
}

/// A fatal CLI failure with structured context and an underlying cause.
///
/// The exit code is chosen by `main`; this type only carries information.
pub struct CliError {
    /// The user-facing summary line.
    message: String,
    /// Structured context rendered as miette help (section 15).
    help: Option<String>,
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl fmt::Debug for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CliError")
            .field("message", &self.message)
            .field("help", &self.help)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl miette::Diagnostic for CliError {
    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.help
            .as_deref()
            .map(|help| Box::new(help) as Box<dyn fmt::Display>)
    }
}

/// Discovers the repository from the current directory and runs one command.
pub fn run(cli: &Cli) -> Result<CommandOutcome, CliError> {
    let start = std::env::current_dir().map_err(|source| CliError {
        message: "could not determine the current directory".to_owned(),
        help: None,
        source: Box::new(source),
    })?;
    let engine = Engine::discover(&start).map_err(engine_failure)?;

    match &cli.command {
        CliCommand::Show {
            mode,
            revision,
            paths,
            areas,
        } => {
            let selection = selection_for(paths, areas, &start, &engine)?;
            let files = engine
                .show(revision, (*mode).into(), &selection)
                .map_err(engine_failure)?;
            output::write_document(DocumentKind::Show, &show_document(&files), cli.color)
                .map_err(output_failure)?;
            Ok(CommandOutcome::Success)
        }
        CliCommand::Diff {
            mode,
            base,
            target,
            paths,
            areas,
            lens,
            decision_provider,
            decision_model,
            decision_endpoint,
            dry_run,
            accept_disclosure,
        } => {
            let selection = selection_for(paths, areas, &start, &engine)?;
            let diffs = engine
                .diff(base, target, (*mode).into(), &selection)
                .map_err(engine_failure)?;

            match lens {
                None => {
                    let (old, new) = split_diff(diffs);
                    output::write_document(
                        DocumentKind::Diff,
                        &diff_document(&old, &new),
                        cli.color,
                    )
                    .map_err(output_failure)?;
                    Ok(CommandOutcome::Success)
                }
                Some(LensChoice::Review) => review_diff(
                    diffs,
                    *decision_provider,
                    decision_model.as_deref(),
                    decision_endpoint.as_deref(),
                    *dry_run,
                    *accept_disclosure,
                    cli.color,
                ),
            }
        }
        #[cfg(feature = "tui")]
        CliCommand::Tui { command } => {
            run_tui(engine, command, &start, cli.icons).map(|()| CommandOutcome::Success)
        }
    }
}

/// Runs the review lens over the engine diffs and writes the review document.
///
/// The canonical diff is rendered from the same projections the legacy path
/// uses, so it appears verbatim above the annotations. Clap guarantees that
/// `--decision-provider` accompanies `--lens`. Before any provider is built,
/// the plan is summarized: a dry run prints it and stops, a remote route must
/// pass the acknowledgement gate, and local routes print a distinct notice.
/// The offline test provider prints nothing.
fn review_diff(
    diffs: Vec<FileDiff>,
    provider_choice: Option<DecisionProviderChoice>,
    model: Option<&str>,
    endpoint: Option<&str>,
    dry_run: bool,
    accept_disclosure: bool,
    color: ColorChoice,
) -> Result<CommandOutcome, CliError> {
    let provider_choice =
        provider_choice.expect("clap requires `--decision-provider` whenever `--lens` is present");
    let route = provider_choice.route();

    // Validate and parse the endpoint before building any state, so a malformed
    // URL is a typed error rather than something a provider sees.
    let host = match endpoint {
        Some(url) => Some(disclosure::endpoint_host(url).map_err(endpoint_failure)?),
        None => None,
    };

    let lens = ReviewLens::new(DEFAULT_MAX_STATE_BYTES).map_err(review_failure)?;
    let plan = disclosure::review_plan(
        provider_choice.label(),
        route,
        host.as_deref(),
        model,
        &lens,
        &diffs,
    );

    if dry_run {
        // A dry run never builds a provider and never transmits state.
        output::write_plain(&disclosure::render_plan(&plan, true)).map_err(output_failure)?;
        return Ok(CommandOutcome::Success);
    }

    match route {
        ProviderRoute::Remote => {
            let interactive = std::io::stderr().is_terminal();
            let accepted = accept_disclosure || env_accepts_disclosure();
            disclosure::check_disclosure(provider_choice.label(), route, interactive, accepted)
                .map_err(disclosure_failure)?;
            write_disclosure(&plan)?;
        }
        // A local provider needs no acknowledgement but is still visibly
        // distinct from an offline one.
        ProviderRoute::Local => write_disclosure(&plan)?,
        // The deterministic offline provider stays byte-for-byte silent so the
        // existing end-to-end output is unchanged.
        ProviderRoute::LocalTest => {}
    }

    let provider =
        decision::build_provider(provider_choice, model, endpoint).map_err(provider_failure)?;

    // `diff_document` consumes owned projections and re-sorts by path; the
    // clone keeps the engine-ordered diffs for per-file review.
    let (old, new) = split_diff(diffs.clone());
    let canonical_diff = diff_document(&old, &new);

    let mut outcomes = Vec::new();
    for diff in &diffs {
        outcomes.extend(lens.review_file(provider.as_ref(), diff));
    }

    let document = review::review_document(
        provider_choice.label(),
        &canonical_diff,
        &outcomes,
        &ReviewRenderOptions::default(),
    );
    output::write_document(DocumentKind::Review, &document, color).map_err(output_failure)?;

    let failed = outcomes
        .iter()
        .any(|outcome| matches!(outcome, ReviewOutcome::Failed { .. }));
    Ok(if failed {
        CommandOutcome::ReviewFailed
    } else {
        CommandOutcome::Success
    })
}

/// Writes the disclosure form of a plan to stderr.
///
/// The disclosure is deliberately never written to stdout, so a redirected
/// review document stays a valid document.
fn write_disclosure(plan: &disclosure::ReviewPlan) -> Result<(), CliError> {
    let text = disclosure::render_plan(plan, false);
    let mut stderr = io::stderr().lock();
    stderr
        .write_all(text.as_bytes())
        .and_then(|()| stderr.flush())
        .map_err(output_failure)
}

/// Whether `OWNAI_ACCEPT_DISCLOSURE` selects one of the truthy spellings.
///
/// The comparison is ASCII case-insensitive, matching the documented values.
fn env_accepts_disclosure() -> bool {
    std::env::var("OWNAI_ACCEPT_DISCLOSURE").is_ok_and(|value| {
        value.eq_ignore_ascii_case("1")
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
    })
}

/// Splits engine diffs into the two projection slices the document renderer
/// consumes. `diff_document` re-sorts by path, so only the path set matters.
fn split_diff(diffs: Vec<FileDiff>) -> (Vec<ProjectedFile>, Vec<ProjectedFile>) {
    let mut old = Vec::new();
    let mut new = Vec::new();
    for diff in diffs {
        if let Some(file) = diff.old {
            old.push(file);
        }
        if let Some(file) = diff.new {
            new.push(file);
        }
    }
    (old, new)
}

/// Runs the terminal frontend for one `tui` subcommand.
#[cfg(feature = "tui")]
fn run_tui(
    engine: Engine,
    command: &TuiCommand,
    start: &Path,
    icons: Option<IconChoice>,
) -> Result<(), CliError> {
    let (request, scope_label) = match command {
        TuiCommand::Show {
            mode,
            revision,
            paths,
            areas,
        } => {
            let selection = selection_for(paths, areas, start, &engine)?;
            let label = scope_label(&selection);
            (
                ownai_tui::LoadRequest::Show {
                    revision: revision.clone(),
                    mode: (*mode).into(),
                    selection,
                },
                label,
            )
        }
        TuiCommand::Diff {
            mode,
            base,
            target,
            paths,
            areas,
        } => {
            let selection = selection_for(paths, areas, start, &engine)?;
            let label = scope_label(&selection);
            (
                ownai_tui::LoadRequest::Diff {
                    base: base.clone(),
                    target: target.clone(),
                    mode: (*mode).into(),
                    selection,
                },
                label,
            )
        }
    };

    let options = ownai_tui::TuiOptions {
        request,
        scope_label,
        icons: resolve_icons(icons),
    };
    ownai_tui::run(engine, options).map_err(tui_failure)
}

/// Resolves the icon style: an explicit `--icons` wins, then `OWNAI_ICONS`,
/// then no icons. Unknown environment values fall back to no icons.
#[cfg(feature = "tui")]
fn resolve_icons(choice: Option<IconChoice>) -> ownai_tui::IconStyle {
    match choice {
        Some(IconChoice::Nerd) => ownai_tui::IconStyle::Nerd,
        Some(IconChoice::None) => ownai_tui::IconStyle::None,
        None => match std::env::var("OWNAI_ICONS").ok().as_deref() {
            Some("nerd") => ownai_tui::IconStyle::Nerd,
            _ => ownai_tui::IconStyle::None,
        },
    }
}

/// A short label for the initial scope, shown in the status bar.
#[cfg(feature = "tui")]
fn scope_label(selection: &Selection) -> String {
    if selection.groups().is_empty() {
        return "all".to_owned();
    }
    selection
        .groups()
        .iter()
        .map(SelectionGroup::label)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Resolves a command's `--path`/`--area` arguments into the selection its
/// projection will use.
///
/// Only an `--area` invocation may touch `.ownai.toml`, so a malformed config
/// can never break `--path` or unscoped runs.
fn selection_for(
    paths: &[OsString],
    areas: &[String],
    cwd: &Path,
    engine: &Engine,
) -> Result<Selection, CliError> {
    if !areas.is_empty() {
        let area_set = engine.load_areas().map_err(engine_failure)?;
        // Resolve first so an unknown or empty area produces the same
        // diagnostic as before, then keep the structured groups for existence
        // checking.
        PathSelection::Areas(areas.to_vec())
            .resolve(&area_set)
            .map_err(|error| path_selection_failure(error, &area_set))?;

        let mut groups = Vec::with_capacity(areas.len());
        for name in areas {
            let Some(area) = area_set.get(name) else {
                continue;
            };
            groups.push(SelectionGroup::Area {
                name: name.clone(),
                paths: area.paths.clone(),
            });
        }
        return Selection::new(groups).map_err(|error| engine_failure(error.into()));
    }

    let path_selection =
        pathspec::build_selection(paths, cwd, engine.root()).map_err(path_arg_failure)?;
    let scope = path_selection
        .resolve(&AreaSet::default())
        .map_err(|error| path_selection_failure(error, &AreaSet::default()))?;
    let groups = scope
        .paths()
        .iter()
        .map(|path| SelectionGroup::Path {
            label: path.to_string(),
            path: path.clone(),
        })
        .collect();
    Selection::new(groups).map_err(|error| engine_failure(error.into()))
}

/// Converts an engine failure into the CLI's reportable error.
///
/// The underlying error is unwrapped rather than nested so the diagnostic's
/// cause chain is identical to the one produced before the engine extraction.
fn engine_failure(error: EngineError) -> CliError {
    match error {
        EngineError::Git { source, revision } => git_failure_for(source, revision.as_deref()),
        EngineError::Config(source) => config_failure(source),
        EngineError::Projection { context, source } => projection_failure(source, *context),
        EngineError::UnsatisfiedSelection { revisions, missing } => {
            let error = MissingPaths { missing };
            let help = error.help(&revisions);
            CliError {
                message: error.to_string(),
                help: Some(help),
                source: Box::new(error),
            }
        }
        EngineError::Selection(source) => CliError {
            message: source.to_string(),
            help: None,
            source: Box::new(source),
        },
    }
}

#[cfg(feature = "tui")]
fn tui_failure<T>(error: T) -> CliError
where
    T: Error + Send + Sync + 'static,
{
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

/// Formats a Git failure, filling in the user revision when the underlying
/// error does not already name it (for example a blob read or tree traversal).
fn git_failure_for(error: GitError, revision: Option<&str>) -> CliError {
    let mut context = git_context(&error);
    if context.revision.is_none() {
        context.revision = revision.map(str::to_owned);
    }
    let help = context_help(&context);
    CliError {
        message: git_message(&error).to_owned(),
        help,
        source: Box::new(error),
    }
}

fn output_failure(source: io::Error) -> CliError {
    CliError {
        message: "could not write output".to_owned(),
        help: None,
        source: Box::new(source),
    }
}

/// A decision provider that could not be constructed.
fn provider_failure(error: ProviderBuildError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

/// A review lens that could not be constructed or evaluated.
fn review_failure(error: ReviewError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

/// A decision endpoint URL that could not be validated.
fn endpoint_failure(error: EndpointError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

/// A remote review that was not acknowledged.
fn disclosure_failure(error: DisclosureError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

fn path_arg_failure(error: PathArgError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

/// Selected paths or areas that name nothing in the revision(s) being
/// projected.
///
/// Reported together so one invocation lists every group that must be fixed,
/// rather than making the user correct them one failure at a time.
#[derive(Debug, thiserror::Error)]
#[error("a selected path or area does not exist in the projected revision")]
struct MissingPaths {
    missing: Vec<SelectionGroup>,
}

impl MissingPaths {
    /// The structured help body: the revision label followed by one
    /// `path:`/`area:` line per missing group.
    fn help(&self, revisions: &str) -> String {
        let mut lines = Vec::with_capacity(self.missing.len() + 1);
        lines.push(format!("revision: {revisions}"));
        for group in &self.missing {
            lines.push(format!("{}: {}", group.kind_label(), group.label()));
        }
        lines.join("\n")
    }
}

fn projection_failure(error: ProjectionError, context: DiagnosticContext) -> CliError {
    let help = context_help(&context);
    CliError {
        message: "a supported source file could not be projected".to_owned(),
        help,
        source: Box::new(error),
    }
}

fn config_failure(error: ConfigError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
}

/// The area seam is reachable through `--area`; an unknown name lists the
/// defined areas so a typo is correctable without opening the config file.
fn path_selection_failure(error: PathSelectionError, areas: &AreaSet) -> CliError {
    let help = match &error {
        PathSelectionError::UnknownArea { .. } => {
            let names: Vec<&str> = areas.names().collect();
            if names.is_empty() {
                Some("no areas are defined in `.ownai.toml`".to_owned())
            } else {
                Some(format!("known areas: {}", names.join(", ")))
            }
        }
        PathSelectionError::EmptyArea { .. } => None,
    };
    CliError {
        message: error.to_string(),
        help,
        source: Box::new(error),
    }
}

/// Extracts the structured fields a [`GitError`] knows about so diagnostics can
/// show them as discrete context rather than buried in a message (section 15).
fn git_context(error: &GitError) -> DiagnosticContext {
    match error {
        GitError::RepositoryNotFound { start, .. } => DiagnosticContext {
            repository: Some(start.clone()),
            ..DiagnosticContext::default()
        },
        GitError::RevisionNotFound {
            repository,
            revision,
            ..
        }
        | GitError::AmbiguousRevision {
            repository,
            revision,
            ..
        }
        | GitError::NotPeelable {
            repository,
            revision,
            ..
        }
        | GitError::RevisionRange {
            repository,
            revision,
        } => DiagnosticContext {
            repository: Some(repository.clone()),
            revision: Some(revision.clone()),
            ..DiagnosticContext::default()
        },
        GitError::ObjectRead { repository, .. }
        | GitError::TreeTraversal { repository, .. }
        | GitError::InvalidRepoPath { repository, .. }
        | GitError::InvalidObjectId { repository, .. } => DiagnosticContext {
            repository: Some(repository.clone()),
            ..DiagnosticContext::default()
        },
    }
}

/// A short summary per failure kind; the cause chain carries the detail.
fn git_message(error: &GitError) -> &'static str {
    match error {
        GitError::RepositoryNotFound { .. } => "could not discover a Git repository",
        GitError::RevisionNotFound { .. } => "could not resolve the requested revision",
        GitError::AmbiguousRevision { .. } => "the requested revision is ambiguous",
        GitError::RevisionRange { .. } => "a revision range is not supported",
        GitError::NotPeelable { .. } => "the requested revision does not name a commit",
        GitError::ObjectRead { .. } => "a Git object could not be read",
        GitError::TreeTraversal { .. } => "a commit tree could not be traversed",
        GitError::InvalidRepoPath { .. } => "a committed entry has an unusable path",
        GitError::InvalidObjectId { .. } => "an object id is invalid",
    }
}

/// Formats the non-empty context fields as help lines.
fn context_help(context: &DiagnosticContext) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(repository) = &context.repository {
        lines.push(format!("repository: {}", repository.display()));
    }
    if let Some(revision) = &context.revision {
        lines.push(format!("revision: {revision}"));
    }
    if let Some(path) = &context.path {
        lines.push(format!("path: {path}"));
    }
    if let Some(language) = context.language {
        lines.push(format!("language: {language:?}"));
    }
    if let Some(range) = &context.range {
        lines.push(format!("range: {}", format_range(range)));
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Core spans are zero-based (section 5); users see one-based positions.
fn format_range(range: &SourceSpan) -> String {
    format!(
        "{}:{}-{}:{}",
        range.start_line + 1,
        range.start_column + 1,
        range.end_line + 1,
        range.end_column + 1
    )
}
