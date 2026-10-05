//! Untracked worktree enumeration.
//!
//! This is the one place Codect reads `.gitignore` and the other exclude
//! sources: committed trees are authoritative, but an untracked file exists
//! only in the worktree, so the standard Git ignore rules decide whether it is
//! part of the working tree. The walk itself is `gix`'s Git-style directory
//! walk, which prunes ignored directories and never descends into nested
//! repositories. No filter, hook, or other repository code is executed.

use bstr::BString;

use crate::GitError;
use crate::repository::GitRepository;
use base::RepoPath;

pub(crate) fn untracked_paths(repo: &GitRepository) -> Result<Vec<RepoPath>, GitError> {
    let repository = repo.location();

    // A bare repository has no worktree, so nothing is untracked.
    if repo.gix().workdir().is_none() {
        return Ok(Vec::new());
    }

    let index = repo
        .gix()
        .index_or_empty()
        .map_err(|source| GitError::IndexRead {
            repository: repository.clone(),
            source: Box::new(source),
        })?;

    // `emit_untracked = Matching` keeps every untracked file individually
    // visible instead of collapsing a fully-untracked directory; ignored
    // entries are not emitted at all, and tracked entries are left to the index.
    let options = repo
        .gix()
        .dirwalk_options()
        .map_err(|source| GitError::UntrackedTraversal {
            repository: repository.clone(),
            source: Box::new(source),
        })?
        .emit_untracked(gix::dir::walk::EmissionMode::Matching)
        .emit_collapsed(Some(gix::dir::walk::CollapsedEntriesEmissionMode::All));

    let walk = repo
        .gix()
        .dirwalk_iter(index, Vec::<BString>::new(), Default::default(), options)
        .map_err(|source| GitError::UntrackedTraversal {
            repository: repository.clone(),
            source: Box::new(source),
        })?;

    let mut paths = Vec::new();
    for item in walk {
        let item = item.map_err(|source| GitError::UntrackedTraversal {
            repository: repository.clone(),
            source: Box::new(source),
        })?;
        let entry = item.entry;

        // Only untracked regular files: directories, symlinks, submodules, and
        // untrackable special files never contribute a projection.
        if entry.status != gix::dir::entry::Status::Untracked
            || entry.disk_kind != Some(gix::dir::entry::Kind::File)
        {
            continue;
        }

        let raw: &[u8] = entry.rela_path.as_ref();
        let path = RepoPath::new(raw).map_err(|source| GitError::InvalidRepoPath {
            repository: repository.clone(),
            path: BString::from(raw.to_vec()),
            source,
        })?;

        if path.language().is_none() {
            continue;
        }

        paths.push(path);
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}
