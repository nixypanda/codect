// The thin command layer.
//
// Everything Git-aware lives in `engine`; this module only builds a
// selection from `argv`, invokes the engine, and renders the result or a
// diagnostic. Keeping the shared pipeline in the engine is what lets the
// terminal frontend reuse it without duplicating behavior.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use base::{
    AreaSet, DiagnosticContext, FileDiff, PathSelection, ProjectedFile, ProjectionError,
    ProjectionMode, RepoPath, Selection, SelectionError, SelectionGroup, SourceSpan, diff_document,
    show_document,
};
use engine::config::ConfigError;
use engine::{DiffBase, Engine, EngineError};
use git::GitError;

#[cfg(feature = "tui")]
use crate::args::IconChoice;
use crate::args::{Cli, Command as CliCommand, Format};
#[cfg(feature = "tui")]
use crate::args::{TuiCommand, TuiDiffCommand};
use crate::json;
use crate::output::{self, DocumentKind};
use crate::pathspec::{self, PathArgError};

// A fatal CLI failure with structured context and an underlying cause.
//
// The exit code is chosen by `main`; this type only carries information.
pub struct CliError {
    message: String,
    help: Option<String>,
    source: Box<dyn Error + Send + Sync + 'static>,
}

impl CliError {
    // A usage failure that exits `2`, matching `clap`'s own usage errors.
    fn usage(message: String) -> Self {
        Self {
            message,
            help: None,
            source: Box::new(UsageError),
        }
    }

    // Whether this failure is a usage error (exit `2`) rather than a runtime
    // failure (exit `1`). Usage errors are recognized by their source type, so
    // every other constructor stays unchanged.
    pub fn is_usage(&self) -> bool {
        self.source.downcast_ref::<UsageError>().is_some()
    }
}

// The source for a [`CliError::usage`]; the detail is in the message.
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
            merge_base,
            paths,
            areas,
        } => {
            let base_mode = merge_base_mode(base, target, *merge_base)?;
            let selection = selection_for(paths, areas, &start, &engine)?;
            match format {
                Format::Text => {
                    let diffs = engine
                        .diff(base, target, (*mode).into(), &selection, base_mode)
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
                        .diff_snapshot_outlines(base, target, (*mode).into(), &selection, base_mode)
                        .map_err(engine_failure)?;
                    output::write_json(&json::diff_document(
                        base,
                        target,
                        (*mode).into(),
                        &diff,
                        *merge_base,
                    ))
                    .map_err(output_failure)
                }
            }
        }
        #[cfg(feature = "tui")]
        CliCommand::Tui { command } => run_tui(engine, command, &start, cli.icons),
    }
}

// Projects one editor-supplied source file, from stdin or the worktree.
//
// Exactly one `--path` supplies the language and the repository-relative path
// used to build stable keys. The path is resolved with the same lexical
// containment rules as `--path` scoping. `--worktree` additionally refuses a
// symlinked target and checks the fully-resolved path, so a read can never
// leave the repository.
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

    let file = engine::project_source(&path, &bytes, mode).map_err(engine_failure)?;
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

// Resolves the single `--path` that supplies the stdin/worktree context.
//
// `--path` is repeatable for revision scoping, but an editor buffer has
// exactly one language and path context, so more than one is a usage error.
// `--path` must name a single file: a directory scope (including `.`, which
// resolves to `PathSelection::All`) is a usage error rather than a runtime
// read failure.
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

// Whether `path`, resolved against the repository root, is an existing
// directory.
//
// `symlink_metadata` describes the link itself, so a symlinked directory is
// classified as a link (and rejected as such by `--worktree`) rather than as
// the directory it points to.
fn is_directory(root: &Path, path: &RepoPath) -> bool {
    std::fs::symlink_metadata(worktree_path(root, path))
        .is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn read_stdin() -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::stdin().lock().read_to_end(&mut bytes)?;
    Ok(bytes)
}

// A worktree file that cannot supply the editor's source bytes.
//
// `--worktree` is the one path that reads a file the user points at on disk,
// so it is the one place where a symlink could redirect a repository-relative
// read outside the repository. The target is rejected when it is itself a
// symlink (mirroring `.codect.toml`) and the fully-resolved path is checked to
// stay inside the repository, so a symlinked ancestor directory cannot escape
// either.
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

// Reads the worktree file for `path`.
//
// Git paths are raw bytes, so on Unix the path is rebuilt from bytes rather
// than lossily converted. The path is already lexically contained in the
// repository; the symlink and resolution checks here close the remaining gap
// between a lexical path and the bytes actually read.
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

// Splits engine diffs into the two projection slices the document renderer
// consumes. `diff_document` re-sorts by path, so only the path set matters.
fn split_diff(diffs: Vec<FileDiff>) -> (Vec<ProjectedFile>, Vec<ProjectedFile>) {
    let mut old = Vec::new();
    let mut new = Vec::new();
    for diff in diffs {
        match diff {
            FileDiff::Deleted { old: file } => old.push(file),
            FileDiff::Added { new: file } => new.push(file),
            FileDiff::Modified {
                old: old_file,
                new: new_file,
            } => {
                old.push(old_file);
                new.push(new_file);
            }
        }
    }
    (old, new)
}

// Resolves the base-side mode for `diff`. `--merge-base` needs two commits:
// the index, worktree, and empty snapshots have no merge base, so a snapshot
// side is a usage error rather than a runtime revision failure.
fn merge_base_mode(base: &str, target: &str, merge_base: bool) -> Result<DiffBase, CliError> {
    if !merge_base {
        return Ok(DiffBase::Given);
    }
    if engine::is_snapshot_spec(base) || engine::is_snapshot_spec(target) {
        return Err(CliError::usage(
            "`--merge-base` requires two commit revisions".to_owned(),
        ));
    }
    Ok(DiffBase::MergeBase)
}

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
                tui::LoadRequest::Show {
                    revision: revision.clone(),
                    mode: (*mode).into(),
                    selection,
                },
                label,
            )
        }
        TuiCommand::Diff { command } => {
            let (view, args) = match command {
                TuiDiffCommand::Range { args } => (tui::DiffView::Range, args),
                TuiDiffCommand::Commits { args } => (tui::DiffView::Commits, args),
            };
            if matches!(view, tui::DiffView::Commits)
                && (engine::is_snapshot_spec(&args.base) || engine::is_snapshot_spec(&args.target))
            {
                return Err(CliError::usage(
                    "the commits view walks commit history; `:index`, `:worktree`, and `:empty` \
                     are only valid with `range`"
                        .to_owned(),
                ));
            }
            if args.merge_base && matches!(view, tui::DiffView::Commits) {
                return Err(CliError::usage(
                    "`--merge-base` is only valid with the range view; the commits view walks \
                     first-parent history"
                        .to_owned(),
                ));
            }
            if args.merge_base
                && (engine::is_snapshot_spec(&args.base) || engine::is_snapshot_spec(&args.target))
            {
                return Err(CliError::usage(
                    "`--merge-base` requires two commit revisions".to_owned(),
                ));
            }
            let selection = selection_for(&args.paths, &args.areas, start, &engine)?;
            let label = scope_label(&selection);
            (
                tui::LoadRequest::Diff {
                    base: args.base.clone(),
                    target: args.target.clone(),
                    mode: args.mode.into(),
                    selection,
                    view,
                    merge_base: args.merge_base,
                },
                label,
            )
        }
    };

    let options = tui::TuiOptions {
        request,
        scope_label,
        icons: resolve_icons(icons),
    };
    tui::run(engine, options).map_err(tui_failure)
}

// Resolves the icon style: an explicit `--icons` wins, then `CODECT_ICONS`,
// then no icons. Unknown environment values fall back to no icons.
#[cfg(feature = "tui")]
fn resolve_icons(choice: Option<IconChoice>) -> tui::IconStyle {
    match choice {
        Some(IconChoice::Nerd) => tui::IconStyle::Nerd,
        Some(IconChoice::None) => tui::IconStyle::None,
        None => match std::env::var("CODECT_ICONS").ok().as_deref() {
            Some("nerd") => tui::IconStyle::Nerd,
            _ => tui::IconStyle::None,
        },
    }
}

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

// Resolves a command's `--path`/`--area` arguments into the selection its
// projection will use.
//
// Only an `--area` invocation may touch `.codect.toml`, so a malformed config
// can never break `--path` or unscoped runs. Resolution records one group per
// literal path or named area in a single step, so an unknown or empty area is
// reported while the same call builds the scope.
fn selection_for(
    paths: &[OsString],
    areas: &[String],
    cwd: &Path,
    engine: &Engine,
) -> Result<Selection, CliError> {
    let (path_selection, area_set) = if areas.is_empty() {
        let selection =
            pathspec::build_selection(paths, cwd, engine.root()).map_err(path_arg_failure)?;
        (selection, AreaSet::default())
    } else {
        (
            PathSelection::Areas(areas.to_vec()),
            engine.load_areas().map_err(engine_failure)?,
        )
    };

    Selection::resolve(&path_selection, &area_set)
        .map_err(|error| selection_failure(error, &area_set))
}

// Converts an engine failure into the CLI's reportable error.
//
// The underlying error is unwrapped rather than nested so the diagnostic's
// cause chain is identical to the one produced before the engine extraction.
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

// Formats a Git failure, filling in the user revision when the underlying
// error does not already name it (for example a blob read or tree traversal).
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

// Selected paths or areas that name nothing in the revision(s) being
// projected.
//
// Reported together so one invocation lists every group that must be fixed,
// rather than making the user correct them one failure at a time.
#[derive(Debug, thiserror::Error)]
#[error("a selected path or area does not exist in the projected revision")]
struct MissingPaths {
    missing: Vec<SelectionGroup>,
}

impl MissingPaths {
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

// The area seam is reachable through `--area`; an unknown name lists the
// defined areas so a typo is correctable without opening the config file. An
// unknown area is now the only way resolution can fail, so the list is always
// the right help.
fn selection_failure(error: SelectionError, areas: &AreaSet) -> CliError {
    let names: Vec<&str> = areas.names().collect();
    let help = if names.is_empty() {
        Some("no areas are defined in `.codect.toml`".to_owned())
    } else {
        Some(format!("known areas: {}", names.join(", ")))
    };
    CliError {
        message: error.to_string(),
        help,
        source: Box::new(error),
    }
}

// Extracts the structured fields a [`GitError`] knows about so diagnostics can
// show them as discrete context rather than buried in a message (section 15).
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
        | GitError::CommitDecode { repository, .. }
        | GitError::TreeTraversal { repository, .. }
        | GitError::InvalidRepoPath { repository, .. }
        | GitError::InvalidObjectId { repository, .. }
        | GitError::NoMergeBase { repository, .. }
        | GitError::MergeBase { repository, .. } => DiagnosticContext {
            repository: Some(repository.clone()),
            ..DiagnosticContext::default()
        },
        GitError::IndexRead { repository, .. } => DiagnosticContext {
            repository: Some(repository.clone()),
            ..DiagnosticContext::default()
        },
        GitError::UntrackedTraversal { repository, .. } => DiagnosticContext {
            repository: Some(repository.clone()),
            ..DiagnosticContext::default()
        },
        GitError::NotFirstParentAncestor { .. } => DiagnosticContext::default(),
    }
}

// A short summary per failure kind; the cause chain carries the detail.
fn git_message(error: &GitError) -> &'static str {
    match error {
        GitError::RepositoryNotFound { .. } => "could not discover a Git repository",
        GitError::RevisionNotFound { .. } => "could not resolve the requested revision",
        GitError::AmbiguousRevision { .. } => "the requested revision is ambiguous",
        GitError::RevisionRange { .. } => "a revision range is not supported",
        GitError::NotPeelable { .. } => "the requested revision does not name a commit",
        GitError::ObjectRead { .. } => "a Git object could not be read",
        GitError::CommitDecode { .. } => "a Git commit could not be decoded",
        GitError::NotFirstParentAncestor { .. } => {
            "the base is not on the target's first-parent chain"
        }
        GitError::TreeTraversal { .. } => "a commit tree could not be traversed",
        GitError::InvalidRepoPath { .. } => "a committed entry has an unusable path",
        GitError::InvalidObjectId { .. } => "an object id is invalid",
        GitError::NoMergeBase { .. } => "the two commits have no merge base",
        GitError::MergeBase { .. } => "a merge base could not be computed",
        GitError::IndexRead { .. } => "the Git index could not be read",
        GitError::UntrackedTraversal { .. } => "untracked files could not be enumerated",
    }
}

fn context_help(context: &DiagnosticContext) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(repository) = &context.repository {
        lines.push(format!("repository: {}", repository.display()));
    }
    if let Some(revision) = &context.revision {
        lines.push(format!("revision: {revision}"));
    }
    if let Some(location) = &context.location {
        lines.push(format!("path: {}", location.path.path()));
        lines.push(format!("language: {:?}", location.path.language()));
        if let Some(range) = &location.range {
            lines.push(format!("range: {}", format_range(range)));
        }
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

// Core spans are zero-based (section 5); users see one-based positions.
fn format_range(range: &SourceSpan) -> String {
    format!(
        "{}:{}-{}:{}",
        range.start_line() + 1,
        range.start_column() + 1,
        range.end_line() + 1,
        range.end_column() + 1
    )
}
