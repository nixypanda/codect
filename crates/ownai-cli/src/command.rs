//! The thin command layer.
//!
//! Everything Git-aware lives in `ownai-engine`; this module only builds a
//! selection from `argv`, invokes the engine, and renders the result or a
//! diagnostic. Keeping the shared pipeline in the engine is what lets the
//! terminal frontend reuse it without duplicating behavior.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use ownai_core::{
    AreaSet, DiagnosticContext, PathSelection, PathSelectionError, ProjectedFile, ProjectionError,
    ProjectionMode, RepoPath, SourceSpan, diff_document, show_document,
};
use ownai_engine::config::ConfigError;
use ownai_engine::{Engine, EngineError, FileDiff, Selection, SelectionGroup};
use ownai_git::GitError;

#[cfg(feature = "tui")]
use crate::args::IconChoice;
#[cfg(feature = "tui")]
use crate::args::TuiCommand;
use crate::args::{Cli, Command as CliCommand, Format};
use crate::json;
use crate::output::{self, DocumentKind};
use crate::pathspec::{self, PathArgError};

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

impl CliError {
    /// A usage failure that exits `2`, matching `clap`'s own usage errors.
    fn usage(message: String) -> Self {
        Self {
            message,
            help: None,
            source: Box::new(UsageError),
        }
    }

    /// Whether this failure is a usage error (exit `2`) rather than a runtime
    /// failure (exit `1`). Usage errors are recognized by their source type, so
    /// every other constructor stays unchanged.
    pub fn is_usage(&self) -> bool {
        self.source.downcast_ref::<UsageError>().is_some()
    }
}

/// The source for a [`CliError::usage`]; the detail is in the message.
#[derive(Debug, thiserror::Error)]
#[error("usage error")]
struct UsageError;

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
pub fn run(cli: &Cli) -> Result<(), CliError> {
    let start = std::env::current_dir().map_err(|source| CliError {
        message: "could not determine the current directory".to_owned(),
        help: None,
        source: Box::new(source),
    })?;
    let engine = Engine::discover(&start).map_err(engine_failure)?;

    match &cli.command {
        CliCommand::Show {
            mode,
            format,
            revision,
            paths,
            areas,
            stdin,
            worktree,
        } => {
            let projection_mode: ProjectionMode = (*mode).into();
            if *stdin || *worktree {
                return run_source_show(
                    *format,
                    projection_mode,
                    paths,
                    *stdin,
                    &start,
                    &engine,
                    cli.color,
                );
            }

            let revision_spec = revision.as_deref().unwrap_or("HEAD");
            let selection = selection_for(paths, areas, &start, &engine)?;
            match format {
                Format::Text => {
                    let files = engine
                        .show(revision_spec, projection_mode, &selection)
                        .map_err(engine_failure)?;
                    output::write_document(DocumentKind::Show, &show_document(&files), cli.color)
                        .map_err(output_failure)
                }
                Format::Json => {
                    let files = engine
                        .show_outlines(revision_spec, projection_mode, &selection)
                        .map_err(engine_failure)?;
                    let document = json::document(
                        json::Input::Revision,
                        Some(revision_spec),
                        projection_mode,
                        &files,
                    );
                    output::write_json(&document).map_err(output_failure)
                }
            }
        }
        CliCommand::Diff {
            mode,
            format,
            base,
            target,
            paths,
            areas,
        } => {
            let selection = selection_for(paths, areas, &start, &engine)?;
            match format {
                Format::Text => {
                    let diffs = engine
                        .diff(base, target, (*mode).into(), &selection)
                        .map_err(engine_failure)?;
                    let (old, new) = split_diff(diffs);
                    output::write_document(
                        DocumentKind::Diff,
                        &diff_document(&old, &new),
                        cli.color,
                    )
                    .map_err(output_failure)
                }
                Format::Json => {
                    let diff = engine
                        .diff_snapshot_outlines(base, target, (*mode).into(), &selection)
                        .map_err(engine_failure)?;
                    output::write_json(&json::diff_document(base, target, (*mode).into(), &diff))
                        .map_err(output_failure)
                }
            }
        }
        #[cfg(feature = "tui")]
        CliCommand::Tui { command } => run_tui(engine, command, &start, cli.icons),
    }
}

/// Projects one editor-supplied source file, from stdin or the worktree.
///
/// Exactly one `--path` supplies the language and the repository-relative path
/// used to build stable keys. The path is resolved with the same lexical
/// containment rules as `--path` scoping. `--worktree` additionally refuses a
/// symlinked target and checks the fully-resolved path, so a read can never
/// leave the repository.
fn run_source_show(
    format: Format,
    mode: ProjectionMode,
    paths: &[OsString],
    from_stdin: bool,
    cwd: &Path,
    engine: &Engine,
    color: crate::args::ColorChoice,
) -> Result<(), CliError> {
    let path = single_source_path(paths, cwd, engine)?;
    let bytes = if from_stdin {
        read_stdin().map_err(stdin_failure)
    } else {
        read_worktree(engine.root(), &path).map_err(worktree_failure)
    }?;

    let file = ownai_engine::project_source(&path, &bytes, mode).map_err(engine_failure)?;
    match format {
        Format::Text => output::write_document(
            DocumentKind::Show,
            &show_document(std::slice::from_ref(&file.projection)),
            color,
        )
        .map_err(output_failure),
        Format::Json => {
            let input = if from_stdin {
                json::Input::Stdin
            } else {
                json::Input::Worktree
            };
            let document = json::document(input, None, mode, std::slice::from_ref(&file));
            output::write_json(&document).map_err(output_failure)
        }
    }
}

/// Resolves the single `--path` that supplies the stdin/worktree context.
///
/// `--path` is repeatable for revision scoping, but an editor buffer has
/// exactly one language and path context, so more than one is a usage error.
/// `--path` must name a single file: a directory scope (including `.`, which
/// resolves to `PathSelection::All`) is a usage error rather than a runtime
/// read failure.
fn single_source_path(
    paths: &[OsString],
    cwd: &Path,
    engine: &Engine,
) -> Result<RepoPath, CliError> {
    if paths.len() != 1 {
        return Err(CliError::usage(
            "`--stdin` and `--worktree` require exactly one `--path`".to_owned(),
        ));
    }

    let selection =
        pathspec::build_selection(paths, cwd, engine.root()).map_err(path_arg_failure)?;
    match selection {
        PathSelection::Literals(mut resolved) if resolved.len() == 1 => {
            let path = resolved.remove(0);
            // A directory cannot be one file's source. The lexical resolution
            // above already rejects `.` and the repository root; this catches a
            // non-root existing directory such as `src`.
            if is_directory(engine.root(), &path) {
                return Err(CliError::usage(
                    "`--stdin` and `--worktree` require `--path` to name one file".to_owned(),
                ));
            }
            Ok(path)
        }
        // `--path .` resolves to the repository root, which is a directory
        // scope rather than a file context.
        _ => Err(CliError::usage(
            "`--stdin` and `--worktree` require `--path` to name one file".to_owned(),
        )),
    }
}

/// Whether `path`, resolved against the repository root, is an existing
/// directory.
///
/// `symlink_metadata` describes the link itself, so a symlinked directory is
/// classified as a link (and rejected as such by `--worktree`) rather than as
/// the directory it points to.
fn is_directory(root: &Path, path: &RepoPath) -> bool {
    std::fs::symlink_metadata(worktree_path(root, path))
        .is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn read_stdin() -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::stdin().lock().read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// A worktree file that cannot supply the editor's source bytes.
///
/// `--worktree` is the one path that reads a file the user points at on disk,
/// so it is the one place where a symlink could redirect a repository-relative
/// read outside the repository. The target is rejected when it is itself a
/// symlink (mirroring `.ownai.toml`) and the fully-resolved path is checked to
/// stay inside the repository, so a symlinked ancestor directory cannot escape
/// either.
#[derive(Debug, thiserror::Error)]
enum WorktreeError {
    #[error("`{path}` is a symlink, which is not followed")]
    Symlink { path: RepoPath },

    #[error("`{path}` resolves outside the repository")]
    OutsideRepository { path: RepoPath },

    #[error("could not read worktree file `{path}`")]
    Read {
        path: RepoPath,
        #[source]
        source: io::Error,
    },
}

/// Reads the worktree file for `path`.
///
/// Git paths are raw bytes, so on Unix the path is rebuilt from bytes rather
/// than lossily converted. The path is already lexically contained in the
/// repository; the symlink and resolution checks here close the remaining gap
/// between a lexical path and the bytes actually read.
fn read_worktree(root: &Path, path: &RepoPath) -> Result<Vec<u8>, WorktreeError> {
    let full = worktree_path(root, path);

    // `symlink_metadata` describes the link itself, which is what decides
    // whether reading the file is allowed at all; `metadata` would silently
    // follow the link and defeat the check.
    let metadata = std::fs::symlink_metadata(&full).map_err(|source| WorktreeError::Read {
        path: path.clone(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(WorktreeError::Symlink { path: path.clone() });
    }

    // Even a non-symlinked target can sit behind a symlinked directory, so the
    // fully-resolved path must still be under the repository.
    let resolved = full.canonicalize().map_err(|source| WorktreeError::Read {
        path: path.clone(),
        source,
    })?;
    let repository = root.canonicalize().map_err(|source| WorktreeError::Read {
        path: path.clone(),
        source,
    })?;
    if !resolved.starts_with(&repository) {
        return Err(WorktreeError::OutsideRepository { path: path.clone() });
    }

    // Read the validated, fully-resolved path rather than the original, so a
    // symlink swapped in after the checks cannot be followed at read time.
    std::fs::read(&resolved).map_err(|source| WorktreeError::Read {
        path: path.clone(),
        source,
    })
}

fn worktree_path(root: &Path, path: &RepoPath) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        root.join(std::ffi::OsStr::from_bytes(path.as_bytes()))
    }
    #[cfg(not(unix))]
    {
        root.join(path.to_string())
    }
}

fn stdin_failure(source: io::Error) -> CliError {
    CliError {
        message: "could not read source from standard input".to_owned(),
        help: None,
        source: Box::new(source),
    }
}

fn worktree_failure(error: WorktreeError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
        source: Box::new(error),
    }
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
        EngineError::WorktreeRead { path, source } => CliError {
            message: format!("could not read worktree file `{path}`"),
            help: None,
            source: Box::new(source),
        },
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
        EngineError::UnsupportedPath { path } => CliError {
            message: "unsupported source path".to_owned(),
            help: None,
            source: Box::new(UnsupportedPath { path }),
        },
    }
}

/// The underlying error for an unsupported explicitly supplied path.
#[derive(Debug, thiserror::Error)]
#[error("no language adapter supports `{path}`")]
struct UnsupportedPath {
    path: RepoPath,
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
        GitError::IndexRead { repository, .. } => DiagnosticContext {
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
        GitError::IndexRead { .. } => "the Git index could not be read",
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
