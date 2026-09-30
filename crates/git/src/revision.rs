//! Revision resolution and commit peeling. Ranges are rejected.

use std::path::Path;

use bstr::BStr;

use crate::GitError;
use crate::repository::{CommitStep, GitRepository, ObjectId, Revision};

/// Traverses only the first parent of each commit. Reading metadata here does
/// not inspect trees or project source, so long histories remain cheap to list.
pub(crate) fn first_parent_steps(
    repo: &GitRepository,
    base: &Revision,
    target: &Revision,
) -> Result<Vec<CommitStep>, GitError> {
    let repository = repo.location();
    let mut current = target.object_id.clone();
    let mut steps = Vec::new();
    while current != base.object_id {
        let commit = repo
            .gix()
            .find_commit(current.to_gix(&repository)?)
            .map_err(|source| GitError::ObjectRead {
                repository: repository.clone(),
                object_id: current.clone(),
                source: Box::new(source),
            })?;
        let Some(parent) = commit.parent_ids().next() else {
            return Err(GitError::NotFirstParentAncestor {
                base_id: base.object_id.clone(),
                target_id: target.object_id.clone(),
            });
        };
        let message = commit
            .message_raw()
            .map_err(|source| GitError::CommitDecode {
                repository: repository.clone(),
                object_id: current.clone(),
                source: Box::new(source),
            })?;
        let message_bytes: &[u8] = message.as_ref();
        let first_line = message_bytes
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let subject = String::from_utf8_lossy(first_line.strip_suffix(b"\r").unwrap_or(first_line))
            .into_owned();
        let parent_id = ObjectId::from_gix(&parent.detach());
        steps.push(CommitStep {
            parent_id: parent_id.clone(),
            commit_id: current,
            subject,
        });
        current = parent_id;
    }
    Ok(steps)
}

pub(crate) fn resolve_commit(repo: &GitRepository, spec: &str) -> Result<Revision, GitError> {
    let repository = repo.location();
    let parsed = repo
        .gix()
        .rev_parse(BStr::new(spec))
        .map_err(|source| classify_parse_error(repository.clone(), spec, source))?;

    let id = match parsed.single() {
        Some(id) => id.detach(),
        None => {
            return Err(GitError::RevisionRange {
                repository,
                revision: spec.to_owned(),
            });
        }
    };

    let commit_id = peel_to_commit(repo, &repository, spec, id)?;
    Ok(Revision {
        object_id: ObjectId::from_gix(&commit_id),
    })
}

/// Distinguishes a genuinely non-peelable object (a tree or blob) from a read
/// failure.
fn peel_to_commit(
    repo: &GitRepository,
    repository: &Path,
    spec: &str,
    id: gix::hash::ObjectId,
) -> Result<gix::hash::ObjectId, GitError> {
    let object = repo
        .gix()
        .find_object(id)
        .map_err(|source| GitError::ObjectRead {
            repository: repository.to_path_buf(),
            object_id: ObjectId::from_gix(&id),
            source: Box::new(source),
        })?;

    match object.peel_to_commit() {
        Ok(commit) => Ok(commit.id),
        Err(source) => {
            let non_peelable = matches!(source, gix::object::peel::to_kind::Error::NotFound { .. });
            if non_peelable {
                Err(GitError::NotPeelable {
                    repository: repository.to_path_buf(),
                    revision: spec.to_owned(),
                    object_id: ObjectId::from_gix(&id),
                    source: Box::new(source),
                })
            } else {
                Err(GitError::ObjectRead {
                    repository: repository.to_path_buf(),
                    object_id: ObjectId::from_gix(&id),
                    source: Box::new(source),
                })
            }
        }
    }
}

/// `gix` reports both "not found" and "ambiguous" as generic error chains, so
/// inspect the retained chain for the disambiguation marker; everything else is
/// treated as not found. Ranges parse successfully and are rejected via
/// [`gix::revision::Spec::single`].
fn classify_parse_error(
    repository: std::path::PathBuf,
    spec: &str,
    source: gix::Error,
) -> GitError {
    let ambiguous = source
        .iter_errors()
        .any(|error| error.to_string().contains("is ambiguous"));

    if ambiguous {
        GitError::AmbiguousRevision {
            repository,
            revision: spec.to_owned(),
            source: Box::new(source),
        }
    } else {
        GitError::RevisionNotFound {
            repository,
            revision: spec.to_owned(),
            source: Box::new(source),
        }
    }
}
