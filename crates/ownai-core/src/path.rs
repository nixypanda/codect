//! Path scoping: prefix sets, repository-defined areas, and the caller's
//! selection.
//!
//! Areas are data only. Reading them from configuration belongs to a higher
//! layer, which keeps core free of file formats and I/O.

use crate::model::RepoPath;

/// A normalized set of repository path prefixes. Empty means "match every
/// path", so the absence of a selection is represented without a separate case.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathScope {
    paths: Vec<RepoPath>,
}

impl PathScope {
    pub fn match_all() -> Self {
        Self { paths: Vec::new() }
    }

    pub fn from_paths(paths: impl IntoIterator<Item = RepoPath>) -> Self {
        let mut paths: Vec<RepoPath> = paths.into_iter().collect();
        paths.sort();
        paths.dedup();
        Self { paths }
    }

    pub fn is_match_all(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn paths(&self) -> &[RepoPath] {
        &self.paths
    }

    /// An empty scope matches everything rather than nothing, so callers never
    /// need a second "no selection" branch.
    pub fn matches(&self, path: &RepoPath) -> bool {
        self.paths.is_empty() || self.paths.iter().any(|prefix| path.is_within(prefix))
    }
}

/// A named, repository-defined group of paths. Areas are data only; this crate
/// never reads a config file.
///
/// The name is non-empty and the path list is non-empty, sorted, and
/// deduplicated; [`Area::new`] establishes all three, so a constructed `Area`
/// always satisfies them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Area {
    name: String,
    paths: Vec<RepoPath>,
}

impl Area {
    /// Rejects an empty name or an empty path list, then sorts and deduplicates
    /// the paths so the stored order is deterministic.
    pub fn new(
        name: impl Into<String>,
        paths: impl IntoIterator<Item = RepoPath>,
    ) -> Result<Self, AreaError> {
        let name = name.into();
        if name.is_empty() {
            return Err(AreaError::EmptyName);
        }

        let mut paths: Vec<RepoPath> = paths.into_iter().collect();
        if paths.is_empty() {
            return Err(AreaError::EmptyPaths { name });
        }
        paths.sort();
        paths.dedup();

        Ok(Self { name, paths })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn paths(&self) -> &[RepoPath] {
        &self.paths
    }
}

/// A lookup of areas by name, sorted by name for deterministic iteration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AreaSet {
    areas: Vec<Area>,
}

impl AreaSet {
    /// Rejects duplicate names so resolution is deterministic.
    pub fn new(areas: impl IntoIterator<Item = Area>) -> Result<Self, AreaError> {
        let mut areas: Vec<Area> = areas.into_iter().collect();
        areas.sort_by(|left, right| left.name.cmp(&right.name));

        // Adjacent comparison is enough because the sort groups equal names.
        for pair in areas.windows(2) {
            if pair[0].name == pair[1].name {
                return Err(AreaError::DuplicateName {
                    name: pair[0].name.clone(),
                });
            }
        }

        Ok(Self { areas })
    }

    pub fn get(&self, name: &str) -> Option<&Area> {
        self.areas
            .binary_search_by(|area| area.name.as_str().cmp(name))
            .ok()
            .map(|index| &self.areas[index])
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.areas.iter().map(|area| area.name.as_str())
    }
}

/// How the caller asked to narrow a projection. The variants make "named areas"
/// and "literal paths" mutually exclusive by construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathSelection {
    All,
    Literals(Vec<RepoPath>),
    Areas(Vec<String>),
}

// Resolution of a `PathSelection` into a concrete scope lives in
// [`crate::selection::Selection::resolve`], which validates areas and records
// the groups an unsatisfied selection needs. `path.rs` stays data-only.

/// A rejected area definition.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AreaError {
    #[error("area name is empty")]
    EmptyName,

    #[error("area `{name}` defines no paths")]
    EmptyPaths { name: String },

    #[error("duplicate area name `{name}`")]
    DuplicateName { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
    }

    fn area(name: &str, paths: &[&str]) -> Area {
        Area::new(name, paths.iter().map(|raw| path(raw))).expect("valid area")
    }

    #[test]
    fn scope_matches_an_exact_file() {
        let scope = PathScope::from_paths([path("src/lib.rs")]);
        assert!(scope.matches(&path("src/lib.rs")));
        assert!(!scope.matches(&path("src/main.rs")));
    }

    #[test]
    fn scope_matches_a_directory_prefix() {
        let scope = PathScope::from_paths([path("src")]);
        assert!(scope.matches(&path("src/lib.rs")));
        assert!(scope.matches(&path("src/nested/deep.rs")));
    }

    #[test]
    fn scope_respects_prefix_component_boundaries() {
        let scope = PathScope::from_paths([path("src")]);
        assert!(!scope.matches(&path("src2/x.rs")));
    }

    #[test]
    fn match_all_matches_everything() {
        let scope = PathScope::match_all();
        assert!(scope.is_match_all());
        assert!(scope.paths().is_empty());
        assert!(scope.matches(&path("anything/at/all.rs")));
    }

    #[test]
    fn from_paths_sorts_and_dedupes() {
        let scope =
            PathScope::from_paths([path("b.rs"), path("a/x.rs"), path("a.rs"), path("a/x.rs")]);

        let ordered: Vec<String> = scope.paths().iter().map(ToString::to_string).collect();
        assert_eq!(ordered, vec!["a.rs", "a/x.rs", "b.rs"]);
    }

    #[test]
    fn scope_matches_non_utf8_paths_exactly() {
        let raw = RepoPath::new(b"src/\xFF/lib.rs".as_slice()).unwrap();
        let other = RepoPath::new(b"src/\xFE/lib.rs".as_slice()).unwrap();
        let scope = PathScope::from_paths([raw.clone()]);

        assert!(scope.matches(&raw));
        assert!(!scope.matches(&other));
    }

    #[test]
    fn area_new_rejects_an_empty_name() {
        let error = Area::new("", [path("src")]).unwrap_err();
        assert_eq!(error, AreaError::EmptyName);
    }

    #[test]
    fn area_new_rejects_an_empty_path_list() {
        let error = Area::new("core", Vec::new()).unwrap_err();
        assert_eq!(
            error,
            AreaError::EmptyPaths {
                name: "core".to_owned()
            }
        );
    }

    #[test]
    fn area_new_sorts_and_dedupes_paths() {
        let area = Area::new("core", [path("b.rs"), path("a.rs"), path("b.rs")]).unwrap();

        assert_eq!(area.name(), "core");
        let ordered: Vec<String> = area.paths().iter().map(ToString::to_string).collect();
        assert_eq!(ordered, vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn area_set_rejects_duplicate_names() {
        let error = AreaSet::new([area("ui", &["src/ui"]), area("ui", &["src/web"])]).unwrap_err();
        assert_eq!(
            error,
            AreaError::DuplicateName {
                name: "ui".to_owned()
            }
        );
    }

    #[test]
    fn area_set_iterates_names_in_sorted_order() {
        let areas = AreaSet::new([area("web", &["src/web"]), area("core", &["src/core"])]).unwrap();
        assert_eq!(areas.names().collect::<Vec<_>>(), vec!["core", "web"]);
        assert!(areas.get("core").is_some());
        assert!(areas.get("ghost").is_none());
    }
}
