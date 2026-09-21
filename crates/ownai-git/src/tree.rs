//! Commit tree traversal and blob reads.

use bstr::BString;

use crate::GitError;
use crate::repository::{GitRepository, ObjectId, Revision, SourceEntry};
use ownai_core::RepoPath;

/// Lists the supported source entries of `revision`'s tree, sorted by raw
/// repository path bytes.
pub(crate) fn source_entries(
    repo: &GitRepository,
    revision: &Revision,
) -> Result<Vec<SourceEntry>, GitError> {
    let repository = repo.location();
    let commit_id = revision.object_id.to_gix(&repository)?;

    let commit = repo
        .gix()
        .find_commit(commit_id)
        .map_err(|source| GitError::ObjectRead {
            repository: repository.clone(),
            object_id: revision.object_id.clone(),
            source: Box::new(source),
        })?;
    let tree = commit.tree().map_err(|source| GitError::TreeTraversal {
        repository: repository.clone(),
        source: Box::new(source),
    })?;

    // Iterative traversal: the work list holds `(tree, prefix)` pairs, so the
    // depth is bounded by available memory rather than the call stack. Git
    // tree nesting is shallow in practice, and an adversarial tree cannot
    // overflow the stack here.
    let mut entries = Vec::new();
    let mut stack = vec![(tree, Vec::new())];

    while let Some((tree, prefix)) = stack.pop() {
        for entry in tree.iter() {
            let entry = entry.map_err(|source| GitError::TreeTraversal {
                repository: repository.clone(),
                source: Box::new(source),
            })?;
            let mode = entry.inner.mode;
            let name = entry.inner.filename.as_ref();
            let object_id = entry.inner.oid.to_owned();

            if mode.is_tree() {
                let child =
                    repo.gix()
                        .find_tree(object_id)
                        .map_err(|source| GitError::TreeTraversal {
                            repository: repository.clone(),
                            source: Box::new(source),
                        })?;
                stack.push((child, join(&prefix, name)));
            } else if mode.is_blob() {
                let path = join(&prefix, name);
                let path = RepoPath::new(&path).map_err(|source| GitError::InvalidRepoPath {
                    repository: repository.clone(),
                    path: BString::from(path.clone()),
                    source,
                })?;

                if path.language().is_none() {
                    continue;
                }

                entries.push(SourceEntry {
                    path,
                    blob_id: ObjectId::from_gix(&object_id),
                    executable: mode.is_executable(),
                });
            }
            // Symlinks, git links/submodules, and any other entry kind are
            // ignored without descending.
        }
    }

    // `Tree::iter` yields Git's canonical tree order, which is not raw path
    // byte order across directories. Sort explicitly for deterministic output;
    // `RepoPath` orders by raw bytes.
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// Reports whether `path` names a tree or blob in `revision`'s tree.
pub(crate) fn path_exists(
    repo: &GitRepository,
    revision: &Revision,
    path: &RepoPath,
) -> Result<bool, GitError> {
    let repository = repo.location();
    let commit_id = revision.object_id.to_gix(&repository)?;

    let commit = repo
        .gix()
        .find_commit(commit_id)
        .map_err(|source| GitError::ObjectRead {
            repository: repository.clone(),
            object_id: revision.object_id.clone(),
            source: Box::new(source),
        })?;
    let mut tree = commit.tree().map_err(|source| GitError::TreeTraversal {
        repository: repository.clone(),
        source: Box::new(source),
    })?;

    // Each component is looked up in exactly one tree level, so a shared prefix
    // such as `src2` never satisfies `src` the way a textual prefix would.
    let components: Vec<&[u8]> = path.as_bytes().split(|&byte| byte == b'/').collect();
    for (index, component) in components.iter().enumerate() {
        let mut child = None;
        for entry in tree.iter() {
            let entry = entry.map_err(|source| GitError::TreeTraversal {
                repository: repository.clone(),
                source: Box::new(source),
            })?;
            let name: &[u8] = entry.inner.filename.as_ref();
            if name == *component {
                child = Some((entry.inner.mode, entry.inner.oid.to_owned()));
                break;
            }
        }

        let Some((mode, object_id)) = child else {
            return Ok(false);
        };

        if index + 1 == components.len() {
            // Only tree and blob entries can hold projectable sources; a
            // symlink or git link names a path but never contributes one.
            return Ok(mode.is_tree() || mode.is_blob());
        }

        if !mode.is_tree() {
            // A blob has no children, so the remaining components cannot exist.
            return Ok(false);
        }

        tree = repo
            .gix()
            .find_tree(object_id)
            .map_err(|source| GitError::TreeTraversal {
                repository: repository.clone(),
                source: Box::new(source),
            })?;
    }

    // `RepoPath` rejects empty input, so every path has at least one component.
    Ok(false)
}

/// Reads the exact bytes of the blob `id`.
pub(crate) fn read_blob(repo: &GitRepository, id: &ObjectId) -> Result<Vec<u8>, GitError> {
    let repository = repo.location();
    let blob_id = id.to_gix(&repository)?;
    let mut blob = repo
        .gix()
        .find_blob(blob_id)
        .map_err(|source| GitError::ObjectRead {
            repository,
            object_id: id.clone(),
            source: Box::new(source),
        })?;
    Ok(blob.take_data())
}

fn join(prefix: &[u8], name: &[u8]) -> Vec<u8> {
    let mut path = Vec::with_capacity(prefix.len() + name.len() + 1);
    if !prefix.is_empty() {
        path.extend_from_slice(prefix);
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}
