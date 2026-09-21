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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Area {
    pub name: String,
    pub paths: Vec<RepoPath>,
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

impl PathSelection {
    /// Resolves to a concrete scope. `All` and an empty literal list both match
    /// everything, so callers never need a second "no selection" branch.
    pub fn resolve(&self, areas: &AreaSet) -> Result<PathScope, PathSelectionError> {
        match self {
            Self::All => Ok(PathScope::match_all()),
            Self::Literals(paths) => Ok(PathScope::from_paths(paths.iter().cloned())),
            Self::Areas(names) => {
                let mut paths = Vec::new();
                for name in names {
                    let area = areas
                        .get(name)
                        .ok_or_else(|| PathSelectionError::UnknownArea { name: name.clone() })?;
                    if area.paths.is_empty() {
                        return Err(PathSelectionError::EmptyArea { name: name.clone() });
                    }
                    paths.extend(area.paths.iter().cloned());
                }
                Ok(PathScope::from_paths(paths))
            }
        }
    }
}

/// A rejected area definition.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AreaError {
    #[error("duplicate area name `{name}`")]
    DuplicateName { name: String },
}

/// A selection that cannot be resolved against the defined areas.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PathSelectionError {
    #[error("no area named `{name}` is defined")]
    UnknownArea { name: String },

    #[error("area `{name}` defines no paths")]
    EmptyArea { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
    }

    fn area(name: &str, paths: &[&str]) -> Area {
        Area {
            name: name.to_owned(),
            paths: paths.iter().map(|raw| path(raw)).collect(),
        }
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

    #[test]
    fn resolve_all_is_match_all() {
        let scope = PathSelection::All.resolve(&AreaSet::default()).unwrap();
        assert!(scope.is_match_all());
        assert!(scope.matches(&path("anywhere/x.rs")));
    }

    #[test]
    fn resolve_areas_unions_multiple_areas() {
        let areas = AreaSet::new([
            area("core", &["crates/core"]),
            area("cli", &["crates/cli", "bin"]),
        ])
        .unwrap();

        let scope = PathSelection::Areas(vec!["core".to_owned(), "cli".to_owned()])
            .resolve(&areas)
            .unwrap();

        assert_eq!(scope.paths().len(), 3);
        assert!(scope.matches(&path("crates/core/src/lib.rs")));
        assert!(scope.matches(&path("bin/ownai")));
        assert!(!scope.matches(&path("docs/readme.md")));
    }

    #[test]
    fn resolve_reports_an_unknown_area() {
        let error = PathSelection::Areas(vec!["ghost".to_owned()])
            .resolve(&AreaSet::default())
            .unwrap_err();

        assert_eq!(
            error,
            PathSelectionError::UnknownArea {
                name: "ghost".to_owned()
            }
        );
    }

    #[test]
    fn resolve_reports_an_empty_area() {
        let areas = AreaSet::new([area("empty", &[])]).unwrap();
        let error = PathSelection::Areas(vec!["empty".to_owned()])
            .resolve(&areas)
            .unwrap_err();

        assert_eq!(
            error,
            PathSelectionError::EmptyArea {
                name: "empty".to_owned()
            }
        );
    }
}
