//! The Git-aware pipeline.
//!
//! Phase 5 kept `ownai-core` Git-free: core only renders projections it is
//! given. Composing `ownai-git`'s snapshot reads with those pure rendering
//! functions therefore belongs here, not in `main.rs` and not in core
//! (TECHNICAL_DESIGN.md sections 3, 7).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::Arc;

use ownai_core::{
    AreaSet, DiagnosticContext, Language, LanguageProjector, PathScope, PathSelectionError,
    ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput, ProjectionMode, RepoPath,
    SourceSpan, decode_source, diff_document, select_projector, show_document,
};
use ownai_git::{GitError, GitRepository, ObjectId, SnapshotRepository, SourceEntry};
use ownai_language_elm::ElmProjector;
use ownai_language_rust::RustProjector;

use crate::args::{Cli, Command as CliCommand};
use crate::output::{self, DocumentKind};
use crate::pathspec::{self, PathArgError};

// ZST projectors are shared as statics so the pipeline never has to own them
// or negotiate a borrow of a longer-lived value.
static ELM_PROJECTOR: ElmProjector = ElmProjector;
static RUST_PROJECTOR: RustProjector = RustProjector;

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
pub fn run(cli: &Cli) -> Result<(), CliError> {
    let start = std::env::current_dir().map_err(|source| CliError {
        message: "could not determine the current directory".to_owned(),
        help: None,
        source: Box::new(source),
    })?;
    let repository = GitRepository::discover(&start).map_err(git_failure)?;
    // A bare repository has no worktree, so relative paths resolve against its
    // root; `gix` reports the bare root as the Git directory.
    let repo_root = repository
        .work_dir()
        .unwrap_or_else(|| repository.git_dir());

    match &cli.command {
        CliCommand::Show {
            mode,
            revision,
            paths,
        } => {
            let scope = scope_for(paths, &start, repo_root)?;
            let document = show(&repository, revision, (*mode).into(), &scope)?;
            output::write_document(DocumentKind::Show, &document, cli.color).map_err(output_failure)
        }
        CliCommand::Diff {
            mode,
            base,
            target,
            paths,
        } => {
            let scope = scope_for(paths, &start, repo_root)?;
            let document = diff(&repository, base, target, (*mode).into(), &scope)?;
            output::write_document(DocumentKind::Diff, &document, cli.color).map_err(output_failure)
        }
    }
}

/// Resolves a command's path arguments into the scope its projection will use.
fn scope_for(paths: &[OsString], cwd: &Path, repo_root: &Path) -> Result<PathScope, CliError> {
    let selection = pathspec::build_selection(paths, cwd, repo_root).map_err(path_arg_failure)?;
    selection
        .resolve(&AreaSet::default())
        .map_err(path_selection_failure)
}

/// Renders the `show` document for `spec` (section 7.1).
fn show(
    repository: &GitRepository,
    spec: &str,
    mode: ProjectionMode,
    scope: &PathScope,
) -> Result<String, CliError> {
    let revision = repository.resolve_commit(spec).map_err(git_failure)?;
    let entries = repository
        .source_entries(&revision)
        .map_err(|error| git_failure_for(error, Some(spec)))?;

    let projectors: [&dyn LanguageProjector; 2] = [&ELM_PROJECTOR, &RUST_PROJECTOR];
    let mut caches = Caches::default();
    let mut files = Vec::new();
    for entry in &entries {
        if !scope.matches(&entry.path) {
            continue;
        }
        if let Some(file) = project_entry(repository, &projectors, &mut caches, spec, entry, mode)?
        {
            files.push(file);
        }
    }

    Ok(show_document(&files))
}

/// Renders the focused `diff` document between `base_spec` and `target_spec`
/// (section 7.2).
fn diff(
    repository: &GitRepository,
    base_spec: &str,
    target_spec: &str,
    mode: ProjectionMode,
    scope: &PathScope,
) -> Result<String, CliError> {
    let base = repository.resolve_commit(base_spec).map_err(git_failure)?;
    let target = repository
        .resolve_commit(target_spec)
        .map_err(git_failure)?;
    let base_entries = repository
        .source_entries(&base)
        .map_err(|error| git_failure_for(error, Some(base_spec)))?;
    let target_entries = repository
        .source_entries(&target)
        .map_err(|error| git_failure_for(error, Some(target_spec)))?;

    let base_map: BTreeMap<&RepoPath, &SourceEntry> = base_entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect();
    let target_map: BTreeMap<&RepoPath, &SourceEntry> = target_entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect();

    // A `BTreeSet` merges the two sides in raw path byte order, which is the
    // order the document will use (sections 7.2, 13).
    let mut paths: BTreeSet<&RepoPath> = base_map.keys().copied().collect();
    paths.extend(target_map.keys().copied());

    let projectors: [&dyn LanguageProjector; 2] = [&ELM_PROJECTOR, &RUST_PROJECTOR];
    let mut caches = Caches::default();
    let mut old_files = Vec::new();
    let mut new_files = Vec::new();

    for path in paths {
        if !scope.matches(path) {
            continue;
        }
        let old = base_map.get(path).copied();
        let new = target_map.get(path).copied();

        // Skip identical blobs before reading or projecting either side. This
        // is the main performance rule of section 17 and is also what makes an
        // unchanged file invisible.
        if let (Some(old), Some(new)) = (old, new)
            && old.blob_id == new.blob_id
        {
            continue;
        }

        if let Some(entry) = old
            && let Some(file) =
                project_entry(repository, &projectors, &mut caches, base_spec, entry, mode)?
        {
            old_files.push(file);
        }
        if let Some(entry) = new
            && let Some(file) = project_entry(
                repository,
                &projectors,
                &mut caches,
                target_spec,
                entry,
                mode,
            )?
        {
            new_files.push(file);
        }
    }

    Ok(diff_document(&old_files, &new_files))
}

/// Per-command caches; never persisted (section 17).
#[derive(Default)]
struct Caches {
    blobs: HashMap<ObjectId, Arc<[u8]>>,
    projections: HashMap<ObjectId, Vec<CachedProjection>>,
}

struct CachedProjection {
    language: Language,
    mode: ProjectionMode,
    items: Vec<ProjectedItem>,
}

/// Reads, decodes, and projects one entry, reusing the caches.
///
/// Returns `None` for an unsupported path, which is an exclusion rather than a
/// failure (section 15).
fn project_entry(
    repository: &GitRepository,
    projectors: &[&dyn LanguageProjector],
    caches: &mut Caches,
    revision_spec: &str,
    entry: &SourceEntry,
    mode: ProjectionMode,
) -> Result<Option<ProjectedFile>, CliError> {
    let Some(projector) = select_projector(projectors, &entry.path) else {
        return Ok(None);
    };
    let language = projector.language();

    if let Some(items) = lookup_projection(caches, &entry.blob_id, language, mode) {
        // The cache key intentionally excludes the path (section 17). Reusing
        // the items for another path is safe because stable keys and spans are
        // never rendered; only `canonical_text` reaches the document.
        return Ok(Some(ProjectedFile::new(
            entry.path.clone(),
            language,
            items,
        )));
    }

    let bytes = read_blob(repository, caches, &entry.blob_id, revision_spec)?;
    let source = decode_source(&entry.path, language, &bytes)
        .map_err(|error| projection_failure(error, repository, revision_spec))?;
    let projected = projector
        .project(ProjectionInput {
            path: &entry.path,
            source,
            mode,
        })
        .map_err(|error| projection_failure(error, repository, revision_spec))?;

    let items = projected.items().to_vec();
    caches
        .projections
        .entry(entry.blob_id.clone())
        .or_default()
        .push(CachedProjection {
            language,
            mode,
            items: items.clone(),
        });
    Ok(Some(ProjectedFile::new(
        entry.path.clone(),
        language,
        items,
    )))
}

fn lookup_projection(
    caches: &Caches,
    id: &ObjectId,
    language: Language,
    mode: ProjectionMode,
) -> Option<Vec<ProjectedItem>> {
    caches.projections.get(id).and_then(|entries| {
        entries
            .iter()
            .find(|entry| entry.language == language && entry.mode == mode)
            .map(|entry| entry.items.clone())
    })
}

/// Reads each blob at most once per command, sharing it via `Arc` so callers
/// can borrow the bytes while the cache stays mutable for later reads.
fn read_blob(
    repository: &GitRepository,
    caches: &mut Caches,
    id: &ObjectId,
    revision_spec: &str,
) -> Result<Arc<[u8]>, CliError> {
    if let Some(bytes) = caches.blobs.get(id) {
        return Ok(Arc::clone(bytes));
    }
    let bytes: Arc<[u8]> = repository
        .read_blob(id)
        .map_err(|error| git_failure_for(error, Some(revision_spec)))?
        .into();
    caches.blobs.insert(id.clone(), Arc::clone(&bytes));
    Ok(bytes)
}

fn projection_failure(
    error: ProjectionError,
    repository: &GitRepository,
    revision_spec: &str,
) -> CliError {
    let context = DiagnosticContext {
        repository: Some(repository.git_dir().to_path_buf()),
        revision: Some(revision_spec.to_owned()),
        path: Some(error.path().clone()),
        language: Some(error.language()),
        range: error.range().cloned(),
    };
    let help = context_help(&context);
    CliError {
        message: "a supported source file could not be projected".to_owned(),
        help,
        source: Box::new(error),
    }
}

fn git_failure(error: GitError) -> CliError {
    git_failure_for(error, None)
}

/// Like [`git_failure`] but fills in the user revision when the underlying
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

/// The area seam is only reachable once configuration exists; mapping it now
/// keeps the CLI honest about resolving through [`PathSelection`].
fn path_selection_failure(error: PathSelectionError) -> CliError {
    CliError {
        message: error.to_string(),
        help: None,
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
