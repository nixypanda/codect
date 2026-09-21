//! Revision resolution and commit peeling. Ranges are rejected.

use std::path::Path;

use bstr::BStr;

use crate::GitError;
use crate::repository::{GitRepository, ObjectId, Revision};

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
