//! The caller's normalized projection scope.
//!
//! A [`Selection`] pairs one [`PathScope`] with the groups that were used to
//! build it. The scope is derived from the groups, so it can never disagree
//! with them; the groups exist only so an unsatisfied selection can name what
//! the user asked for.
//!
//! [`Selection::resolve`] is the single entry point from a raw
//! [`PathSelection`] plus the repository's [`AreaSet`]: it validates named
//! areas and records one group per literal path or area in one step, so callers
//! never resolve a scope and then rebuild its groups by hand.

use crate::model::RepoPath;
use crate::path::{AreaSet, PathScope, PathSelection};

/// One user-visible unit of selection. A literal path is its own group; a named
/// area is one group however many paths it defines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionGroup {
    Path { label: String, path: RepoPath },
    Area { name: String, paths: Vec<RepoPath> },
}

impl SelectionGroup {
    /// The diagnostic key for this group: `path` or `area`.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Path { .. } => "path",
            Self::Area { .. } => "area",
        }
    }

    /// The label the user wrote on the command line.
    pub fn label(&self) -> &str {
        match self {
            Self::Path { label, .. } => label,
            Self::Area { name, .. } => name,
        }
    }

    /// The repository paths this group contributes to the scope.
    pub fn paths(&self) -> &[RepoPath] {
        match self {
            Self::Path { path, .. } => std::slice::from_ref(path),
            Self::Area { paths, .. } => paths,
        }
    }
}

/// A normalized scope plus the groups it was built from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection {
    scope: PathScope,
    groups: Vec<SelectionGroup>,
}

impl Selection {
    /// Everything: a match-all scope with no existence checks.
    pub fn all() -> Self {
        Self {
            scope: PathScope::match_all(),
            groups: Vec::new(),
        }
    }

    /// Builds a selection from its groups. The scope is the union of every
    /// group's paths, so an empty group list selects everything.
    pub fn new(groups: Vec<SelectionGroup>) -> Result<Self, SelectionError> {
        for group in &groups {
            if group.paths().is_empty() {
                return Err(SelectionError::EmptyGroup {
                    label: group.label().to_owned(),
                });
            }
        }
        Ok(Self::from_groups(groups))
    }

    /// Resolves a raw [`PathSelection`] against the repository's areas in one
    /// step, recording one group per literal path or named area.
    ///
    /// `All` (and an empty literal list) selects everything. Literal paths are
    /// sorted and deduplicated, matching the scope they produce, and each keeps
    /// its escaped display form as its diagnostic label. A named area must
    /// exist and define at least one path.
    pub fn resolve(selection: &PathSelection, areas: &AreaSet) -> Result<Self, SelectionError> {
        match selection {
            PathSelection::All => Ok(Self::all()),
            PathSelection::Literals(paths) => {
                let mut unique: Vec<RepoPath> = paths.clone();
                unique.sort();
                unique.dedup();
                let groups = unique
                    .into_iter()
                    .map(|path| {
                        let label = path.to_string();
                        SelectionGroup::Path { label, path }
                    })
                    .collect();
                Ok(Self::from_groups(groups))
            }
            PathSelection::Areas(names) => {
                let mut groups = Vec::with_capacity(names.len());
                for name in names {
                    let area = areas
                        .get(name)
                        .ok_or_else(|| SelectionError::UnknownArea { name: name.clone() })?;
                    if area.paths.is_empty() {
                        return Err(SelectionError::EmptyArea { name: name.clone() });
                    }
                    groups.push(SelectionGroup::Area {
                        name: name.clone(),
                        paths: area.paths.clone(),
                    });
                }
                Ok(Self::from_groups(groups))
            }
        }
    }

    /// Builds the scope from already-validated groups.
    fn from_groups(groups: Vec<SelectionGroup>) -> Self {
        let scope = PathScope::from_paths(
            groups
                .iter()
                .flat_map(|group| group.paths().iter().cloned()),
        );
        Self { scope, groups }
    }

    pub fn scope(&self) -> &PathScope {
        &self.scope
    }

    pub fn groups(&self) -> &[SelectionGroup] {
        &self.groups
    }
}

/// A selection that cannot be constructed or resolved.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SelectionError {
    #[error("selection group `{label}` defines no paths")]
    EmptyGroup { label: String },

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

    fn area(name: &str, paths: &[&str]) -> crate::path::Area {
        crate::path::Area {
            name: name.to_owned(),
            paths: paths.iter().map(|raw| path(raw)).collect(),
        }
    }

    #[test]
    fn all_matches_everything_and_has_no_groups() {
        let selection = Selection::all();
        assert!(selection.scope().is_match_all());
        assert!(selection.groups().is_empty());
        assert!(selection.scope().matches(&path("anywhere/x.rs")));
    }

    #[test]
    fn a_path_group_scopes_to_that_path() {
        let selection = Selection::new(vec![SelectionGroup::Path {
            label: "src/lib.rs".to_owned(),
            path: path("src/lib.rs"),
        }])
        .unwrap();

        assert!(selection.scope().matches(&path("src/lib.rs")));
        assert!(!selection.scope().matches(&path("src/main.rs")));
        assert_eq!(selection.groups().len(), 1);
        assert_eq!(selection.groups()[0].kind_label(), "path");
        assert_eq!(selection.groups()[0].label(), "src/lib.rs");
    }

    #[test]
    fn an_area_group_scopes_to_all_its_paths() {
        let selection = Selection::new(vec![SelectionGroup::Area {
            name: "frontend".to_owned(),
            paths: vec![path("apps/web"), path("packages/ui")],
        }])
        .unwrap();

        assert!(selection.scope().matches(&path("apps/web/src/App.elm")));
        assert!(selection.scope().matches(&path("packages/ui/src/lib.rs")));
        assert!(!selection.scope().matches(&path("services/api/src/main.rs")));
        assert_eq!(selection.groups()[0].kind_label(), "area");
    }

    #[test]
    fn an_empty_group_is_rejected() {
        let error = Selection::new(vec![SelectionGroup::Area {
            name: "empty".to_owned(),
            paths: Vec::new(),
        }])
        .unwrap_err();

        assert_eq!(
            error,
            SelectionError::EmptyGroup {
                label: "empty".to_owned()
            }
        );
    }

    #[test]
    fn an_empty_group_list_is_match_all() {
        let selection = Selection::new(Vec::new()).unwrap();
        assert!(selection.scope().is_match_all());
    }

    #[test]
    fn non_utf8_path_groups_keep_their_identity() {
        let raw = RepoPath::new(b"src/\xFF/lib.rs".as_slice()).unwrap();
        let selection = Selection::new(vec![SelectionGroup::Path {
            label: raw.to_string(),
            path: raw.clone(),
        }])
        .unwrap();

        assert!(selection.scope().matches(&raw));
    }

    #[test]
    fn resolve_all_is_match_all() {
        let selection = Selection::resolve(&PathSelection::All, &AreaSet::default()).unwrap();
        assert!(selection.scope().is_match_all());
        assert!(selection.scope().matches(&path("anywhere/x.rs")));
        assert!(selection.groups().is_empty());
    }

    #[test]
    fn resolve_literals_keeps_one_sorted_deduplicated_group_per_path() {
        let selection = Selection::resolve(
            &PathSelection::Literals(vec![
                path("b.rs"),
                path("a/x.rs"),
                path("a.rs"),
                path("a/x.rs"),
            ]),
            &AreaSet::default(),
        )
        .unwrap();

        let labels: Vec<&str> = selection
            .groups()
            .iter()
            .map(SelectionGroup::label)
            .collect();
        assert_eq!(labels, vec!["a.rs", "a/x.rs", "b.rs"]);
        assert_eq!(selection.groups()[0].kind_label(), "path");
    }

    #[test]
    fn resolve_areas_unions_multiple_areas() {
        let areas = AreaSet::new([
            area("core", &["crates/core"]),
            area("cli", &["crates/cli", "bin"]),
        ])
        .unwrap();

        let selection = Selection::resolve(
            &PathSelection::Areas(vec!["core".to_owned(), "cli".to_owned()]),
            &areas,
        )
        .unwrap();

        assert_eq!(selection.scope().paths().len(), 3);
        assert!(selection.scope().matches(&path("crates/core/src/lib.rs")));
        assert!(selection.scope().matches(&path("bin/ownai")));
        assert!(!selection.scope().matches(&path("docs/readme.md")));
        assert_eq!(selection.groups()[0].kind_label(), "area");
        assert_eq!(selection.groups()[0].label(), "core");
    }

    #[test]
    fn resolve_reports_an_unknown_area() {
        let error = Selection::resolve(
            &PathSelection::Areas(vec!["ghost".to_owned()]),
            &AreaSet::default(),
        )
        .unwrap_err();

        assert_eq!(
            error,
            SelectionError::UnknownArea {
                name: "ghost".to_owned()
            }
        );
    }

    #[test]
    fn resolve_reports_an_empty_area() {
        let areas = AreaSet::new([area("empty", &[])]).unwrap();
        let error = Selection::resolve(&PathSelection::Areas(vec!["empty".to_owned()]), &areas)
            .unwrap_err();

        assert_eq!(
            error,
            SelectionError::EmptyArea {
                name: "empty".to_owned()
            }
        );
    }
}
