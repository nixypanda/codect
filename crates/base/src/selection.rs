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

use std::borrow::Cow;

use crate::model::RepoPath;
use crate::path::{Area, AreaSet, PathScope, PathSelection};

/// One user-visible unit of selection. A literal path is its own group; a named
/// area is one group however many paths it defines.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionGroup {
    Path(RepoPath),
    Area(Area),
}

impl SelectionGroup {
    /// The diagnostic key for this group: `path` or `area`.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Path(_) => "path",
            Self::Area(_) => "area",
        }
    }

    /// The label the user wrote on the command line. A literal path's label is
    /// its escaped display form; an area's is its name.
    pub fn label(&self) -> Cow<'_, str> {
        match self {
            Self::Path(path) => Cow::Owned(path.to_string()),
            Self::Area(area) => Cow::Borrowed(area.name()),
        }
    }

    /// The repository paths this group contributes to the scope.
    pub fn paths(&self) -> &[RepoPath] {
        match self {
            Self::Path(path) => std::slice::from_ref(path),
            Self::Area(area) => area.paths(),
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
    /// group's paths, so an empty group list selects everything. Every group is
    /// non-empty by construction, so this cannot fail.
    pub fn new(groups: Vec<SelectionGroup>) -> Self {
        let scope = PathScope::from_paths(
            groups
                .iter()
                .flat_map(|group| group.paths().iter().cloned()),
        );
        Self { scope, groups }
    }

    /// Resolves a raw [`PathSelection`] against the repository's areas in one
    /// step, recording one group per literal path or named area.
    ///
    /// `All` (and an empty literal list) selects everything. Literal paths are
    /// sorted and deduplicated, matching the scope they produce, and each keeps
    /// its escaped display form as its diagnostic label. A named area must
    /// exist; it is non-empty by construction.
    pub fn resolve(selection: &PathSelection, areas: &AreaSet) -> Result<Self, SelectionError> {
        match selection {
            PathSelection::All => Ok(Self::all()),
            PathSelection::Literals(paths) => {
                let mut unique: Vec<RepoPath> = paths.clone();
                unique.sort();
                unique.dedup();
                let groups = unique.into_iter().map(SelectionGroup::Path).collect();
                Ok(Self::new(groups))
            }
            PathSelection::Areas(names) => {
                let mut groups = Vec::with_capacity(names.len());
                for name in names {
                    let area = areas
                        .get(name)
                        .ok_or_else(|| SelectionError::UnknownArea { name: name.clone() })?;
                    groups.push(SelectionGroup::Area(area.clone()));
                }
                Ok(Self::new(groups))
            }
        }
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
    #[error("no area named `{name}` is defined")]
    UnknownArea { name: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
    }

    fn area(name: &str, paths: &[&str]) -> crate::path::Area {
        crate::path::Area::new(name, paths.iter().map(|raw| path(raw))).expect("valid area")
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
        let selection = Selection::new(vec![SelectionGroup::Path(path("src/lib.rs"))]);

        assert!(selection.scope().matches(&path("src/lib.rs")));
        assert!(!selection.scope().matches(&path("src/main.rs")));
        assert_eq!(selection.groups().len(), 1);
        assert_eq!(selection.groups()[0].kind_label(), "path");
        assert_eq!(selection.groups()[0].label().to_string(), "src/lib.rs");
    }

    #[test]
    fn an_area_group_scopes_to_all_its_paths() {
        let selection = Selection::new(vec![SelectionGroup::Area(area(
            "frontend",
            &["apps/web", "packages/ui"],
        ))]);

        assert!(selection.scope().matches(&path("apps/web/src/App.elm")));
        assert!(selection.scope().matches(&path("packages/ui/src/lib.rs")));
        assert!(!selection.scope().matches(&path("services/api/src/main.rs")));
        assert_eq!(selection.groups()[0].kind_label(), "area");
    }

    #[test]
    fn an_empty_group_list_is_match_all() {
        let selection = Selection::new(Vec::new());
        assert!(selection.scope().is_match_all());
    }

    #[test]
    fn non_utf8_path_groups_keep_their_identity() {
        let raw = RepoPath::new(b"src/\xFF/lib.rs".as_slice()).unwrap();
        let selection = Selection::new(vec![SelectionGroup::Path(raw.clone())]);

        assert!(selection.scope().matches(&raw));
        assert_eq!(selection.groups()[0].label().to_string(), raw.to_string());
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

        let labels: Vec<String> = selection
            .groups()
            .iter()
            .map(|group| group.label().to_string())
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
        assert!(selection.scope().matches(&path("bin/codect")));
        assert!(!selection.scope().matches(&path("docs/readme.md")));
        assert_eq!(selection.groups()[0].kind_label(), "area");
        assert_eq!(selection.groups()[0].label().to_string(), "core");
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
}
