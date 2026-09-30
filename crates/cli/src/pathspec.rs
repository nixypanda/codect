// Conversion from command-line OS paths to the core path-selection seam.
//
// `base` deliberately knows nothing about the operating system's notion
// of a path, and `git` must not learn about selection, so the CLI owns
// the one place where an `OsString` argument becomes a repository-relative
// `RepoPath` (TECHNICAL_DESIGN.md section 3).
//
// Resolution is purely lexical: the working directory is already resolved by
// the OS and the repository root by `gix` discovery, so comparing their
// components needs no filesystem call and cannot disagree with them. Calling
// `canonicalize` would additionally resolve symlinks, which Git does not, and
// would reject paths that do not exist on disk but do exist in a revision.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use base::{PathSelection, RepoPath, RepoPathError};

// Resolves command-line path arguments into a selection relative to
// `repo_root`.
//
// An empty argument list, and any argument that resolves to the repository
// root itself, both select everything: the root subsumes every other argument.
pub fn build_selection(
    paths: &[OsString],
    cwd: &Path,
    repo_root: &Path,
) -> Result<PathSelection, PathArgError> {
    if paths.is_empty() {
        return Ok(PathSelection::All);
    }

    let root = split_components(&path_bytes(repo_root));
    let base = split_components(&path_bytes(cwd));
    let mut resolved = Vec::with_capacity(paths.len());

    for path in paths {
        let display = path.to_string_lossy().into_owned();
        let bytes = path_arg_bytes(path)?;

        // An absolute argument replaces the working directory; a relative one
        // extends it. Either way `..` may only climb within the repository.
        let mut components = if bytes.first() == Some(&b'/') {
            Vec::new()
        } else {
            base.clone()
        };

        for component in bytes.split(|&byte| byte == b'/') {
            if component.is_empty() || component == b"." {
                continue;
            }
            if component == b".." {
                if components.pop().is_none() {
                    return Err(outside(display, repo_root));
                }
                continue;
            }
            components.push(component.to_vec());
        }

        let Some(relative) = components.strip_prefix(root.as_slice()) else {
            return Err(outside(display, repo_root));
        };
        if relative.is_empty() {
            return Ok(PathSelection::All);
        }

        let repo_path = RepoPath::new(join(relative)).map_err(|source| PathArgError::Invalid {
            path: display,
            source,
        })?;
        resolved.push(repo_path);
    }

    Ok(PathSelection::Literals(resolved))
}

#[derive(Debug, thiserror::Error)]
pub enum PathArgError {
    #[error("path `{path}` is outside repository `{repository}`")]
    OutsideRepository { path: String, repository: PathBuf },

    // Only constructible where `OsStr` is not already raw bytes, but kept in
    // the fixed error surface so callers never see a platform difference.
    #[cfg_attr(unix, allow(dead_code))]
    #[error("path `{path}` is not valid UTF-8")]
    NonUtf8 { path: String },

    #[error("path `{path}` is not a usable repository path")]
    Invalid {
        path: String,
        #[source]
        source: RepoPathError,
    },
}

fn outside(path: String, repository: &Path) -> PathArgError {
    PathArgError::OutsideRepository {
        path,
        repository: repository.to_path_buf(),
    }
}

// Repository paths use `/` on every platform, so components are compared as
// bytes rather than through the host's path semantics.
fn path_bytes(path: &Path) -> Vec<u8> {
    let os = path.as_os_str();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        os.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        os.to_string_lossy().into_owned().into_bytes()
    }
}

// Keeps committed non-UTF-8 paths targetable on Unix; elsewhere the conversion
// is best-effort and a missing UTF-8 form is reported rather than guessed.
fn path_arg_bytes(value: &OsStr) -> Result<Vec<u8>, PathArgError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        Ok(value.as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        value
            .to_str()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| PathArgError::NonUtf8 {
                path: value.to_string_lossy().into_owned(),
            })
    }
}

fn split_components(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes
        .split(|&byte| byte == b'/')
        .filter(|component| !component.is_empty() && *component != b".")
        .map(<[u8]>::to_vec)
        .collect()
}

fn join(components: &[Vec<u8>]) -> Vec<u8> {
    let mut joined = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            joined.push(b'/');
        }
        joined.extend_from_slice(component);
    }
    joined
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn build(paths: &[&str], cwd: &str, repo_root: &str) -> Result<PathSelection, PathArgError> {
        let paths: Vec<OsString> = paths.iter().map(OsString::from).collect();
        build_selection(&paths, Path::new(cwd), Path::new(repo_root))
    }

    fn literal(path: &str) -> RepoPath {
        RepoPath::new(path).expect("valid repository path")
    }

    #[test]
    fn relative_child_resolves_under_the_working_directory() {
        let selection = build(&["foo.rs"], "/repo/src", "/repo").unwrap();
        assert_eq!(
            selection,
            PathSelection::Literals(vec![literal("src/foo.rs")])
        );
    }

    #[test]
    fn empty_input_selects_everything() {
        assert_eq!(
            build(&[], "/repo/src", "/repo").unwrap(),
            PathSelection::All
        );
    }

    #[test]
    fn dot_at_the_repository_root_selects_everything() {
        assert_eq!(build(&["."], "/repo", "/repo").unwrap(), PathSelection::All);
    }

    #[test]
    fn dot_inside_a_subdirectory_selects_that_directory() {
        let selection = build(&["."], "/repo/src", "/repo").unwrap();
        assert_eq!(selection, PathSelection::Literals(vec![literal("src")]));
    }

    #[test]
    fn parent_traversal_climbs_one_level() {
        let selection = build(&[".."], "/repo/src/nested", "/repo").unwrap();
        assert_eq!(selection, PathSelection::Literals(vec![literal("src")]));
    }

    #[test]
    fn parent_traversal_beyond_the_root_is_rejected() {
        let error = build(&[".."], "/repo", "/repo").unwrap_err();
        assert!(matches!(error, PathArgError::OutsideRepository { .. }));
    }

    #[test]
    fn absolute_path_under_the_root_becomes_relative() {
        let selection = build(&["/repo/src/lib.rs"], "/repo", "/repo").unwrap();
        assert_eq!(
            selection,
            PathSelection::Literals(vec![literal("src/lib.rs")])
        );
    }

    #[test]
    fn absolute_path_outside_the_root_is_rejected() {
        let error = build(&["/etc/passwd"], "/repo", "/repo").unwrap_err();
        assert!(matches!(error, PathArgError::OutsideRepository { .. }));
    }

    #[test]
    fn interior_dot_components_are_normalized_away() {
        let selection = build(&["src/./foo.rs"], "/repo", "/repo").unwrap();
        assert_eq!(
            selection,
            PathSelection::Literals(vec![literal("src/foo.rs")])
        );
    }

    #[test]
    fn a_trailing_slash_is_normalized_away() {
        let selection = build(&["src/foo.rs/"], "/repo", "/repo").unwrap();
        assert_eq!(
            selection,
            PathSelection::Literals(vec![literal("src/foo.rs")])
        );
    }

    #[test]
    fn a_path_that_resolves_to_the_root_selects_everything() {
        assert_eq!(
            build(&["src/.."], "/repo", "/repo").unwrap(),
            PathSelection::All
        );
    }
}
