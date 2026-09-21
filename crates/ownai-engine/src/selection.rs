//! The caller's normalized projection scope.
//!
//! A [`Selection`] pairs one [`PathScope`] with the groups that were used to
//! build it. The scope is derived from the groups, so it can never disagree
//! with them; the groups exist only so an unsatisfied selection can name what
//! the user asked for.

use ownai_core::{PathScope, RepoPath};

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
        let scope = PathScope::from_paths(
            groups
                .iter()
                .flat_map(|group| group.paths().iter().cloned()),
        );
        Ok(Self { scope, groups })
    }

    pub fn scope(&self) -> &PathScope {
        &self.scope
    }

    pub fn groups(&self) -> &[SelectionGroup] {
        &self.groups
    }
}

/// A selection that cannot be constructed.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SelectionError {
    #[error("selection group `{label}` defines no paths")]
    EmptyGroup { label: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
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
}
