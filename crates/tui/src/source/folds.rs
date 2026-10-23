//! The source view's fold caches, fold overrides, and syntax highlighting.

use std::collections::BTreeMap;
use std::sync::Arc;

use base::{FoldRegion, RepoPath};

use tui_core::cx::Cx;
use tui_core::render::highlight::{self, StyledLine};
use tui_core::render::theme::Theme;
use tui_core::util::cache::BoundedCache;

use super::SourceView;
use super::filters_dropped_declarations;
use super::rows::VisibleRow;

/// The highlighted lines of the source view's files.
#[derive(Clone, Debug, Default)]
pub struct ShowHighlights {
    cache: BoundedCache<Arc<Vec<StyledLine>>>,
}

impl ShowHighlights {
    pub fn lines(&self, path: &RepoPath) -> Option<&[StyledLine]> {
        self.cache.get(path).map(|lines| lines.as_slice())
    }

    pub fn contains(&self, path: &RepoPath) -> bool {
        self.cache.contains_key(path)
    }

    fn insert(&mut self, path: RepoPath, lines: Vec<StyledLine>) {
        self.cache.insert(path, Arc::new(lines));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The fold regions of the source view's files.
///
/// The cache is keyed by path only. Correctness therefore relies on
/// `update::loaded` clearing it whenever the loaded revision or mode changes;
/// retention is baked into each `FoldRegion`.
#[derive(Clone, Debug, Default)]
pub struct ShowFolds {
    cache: BoundedCache<Arc<Vec<FoldRegion>>>,
}

impl ShowFolds {
    pub fn regions(&self, path: &RepoPath) -> Option<&[FoldRegion]> {
        self.cache.get(path).map(|regions| regions.as_slice())
    }

    pub fn contains(&self, path: &RepoPath) -> bool {
        self.cache.contains_key(path)
    }

    fn insert(&mut self, path: RepoPath, regions: Vec<FoldRegion>) {
        self.cache.insert(path, Arc::new(regions));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The sticky fold overrides, keyed by path and stable region key.
#[derive(Clone, Debug, Default)]
pub struct FoldState {
    overrides: BTreeMap<RepoPath, BTreeMap<String, bool>>,
    version: u64,
}

impl FoldState {
    pub fn is_open(&self, path: &RepoPath, region: &FoldRegion) -> bool {
        self.overrides
            .get(path)
            .and_then(|regions| regions.get(region.stable_key.as_str()))
            .copied()
            .unwrap_or(region.default_open)
    }

    pub fn set(&mut self, path: &RepoPath, stable_key: &str, open: bool) {
        self.overrides
            .entry(path.clone())
            .or_default()
            .insert(stable_key.to_owned(), open);
        self.version = self.version.wrapping_add(1);
    }

    pub(super) fn version(&self) -> u64 {
        self.version
    }
}

/// One fold command's effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Fold {
    Toggle,
    Open,
    Close,
    OpenRecursive,
    CloseRecursive,
    OpenAll,
    CloseAll,
}

/// The fold effect a command name names, if any.
pub(crate) fn fold_command(name: &str) -> Option<Fold> {
    Some(match name {
        "fold.toggle" => Fold::Toggle,
        "fold.open" => Fold::Open,
        "fold.close" => Fold::Close,
        "fold.open_recursive" => Fold::OpenRecursive,
        "fold.close_recursive" => Fold::CloseRecursive,
        "fold.open_all" => Fold::OpenAll,
        "fold.close_all" => Fold::CloseAll,
        _ => return None,
    })
}

impl SourceView {
    pub(crate) fn ensure_folds(&mut self, selected: Option<&RepoPath>) {
        let Some(path) = selected else {
            return;
        };
        if self.folds.contains(path) {
            return;
        }
        let Some(file) = self.files.iter().find(|file| file.path() == path) else {
            return;
        };
        let lines: Vec<&str> = file.source.lines().collect();
        let regions = base::fold_regions(&file.outline, &lines);
        self.folds.insert(path.clone(), regions);
    }

    pub(crate) fn ensure_highlight(&mut self, theme: &Theme, selected: Option<&RepoPath>) {
        let Some(path) = selected else {
            return;
        };
        if self.highlight.contains(path) {
            return;
        }
        let Some(file) = self.files.iter().find(|file| file.path() == path) else {
            return;
        };
        let lines = highlight::highlight(&file.source, file.language(), theme);
        self.highlight.insert(path.clone(), lines);
    }

    pub(crate) fn fold_action(&mut self, action: Fold, cx: &Cx) {
        let selected = cx.workspace.selected.clone();
        let selected = selected.as_ref();
        self.ensure_visible_rows(selected);
        let Some(path) = selected else {
            return;
        };
        let Some(regions) = self.folds.regions(path).map(<[FoldRegion]>::to_vec) else {
            return;
        };
        if regions.is_empty() {
            return;
        }
        // Dropped regions intentionally receive overrides too. Overrides are
        // sticky across mode reloads, and the filtered row builder hides their
        // effects; do not filter this iteration by retention.
        match action {
            Fold::OpenAll => {
                for region in &regions {
                    self.fold_state.set(path, &region.stable_key, true);
                }
            }
            Fold::CloseAll => {
                for region in &regions {
                    self.fold_state.set(path, &region.stable_key, false);
                }
            }
            _ => {
                let Some(target) = self.target_region(&regions) else {
                    return;
                };
                match action {
                    Fold::Toggle => {
                        let open = !self.fold_state.is_open(path, &regions[target]);
                        self.fold_state.set(path, &regions[target].stable_key, open);
                    }
                    Fold::Open => {
                        self.fold_state.set(path, &regions[target].stable_key, true);
                    }
                    Fold::Close => {
                        self.fold_state
                            .set(path, &regions[target].stable_key, false);
                    }
                    Fold::OpenRecursive => {
                        for key in subtree_keys(&regions, target) {
                            self.fold_state.set(path, &key, true);
                        }
                    }
                    Fold::CloseRecursive => {
                        for key in subtree_keys(&regions, target) {
                            self.fold_state.set(path, &key, false);
                        }
                    }
                    Fold::OpenAll | Fold::CloseAll => {}
                }
            }
        }
        self.ensure_visible_rows(selected);
        self.reanchor_cursor(selected);
        self.clamp_view(cx.size.height, selected);
    }

    fn target_region(&self, regions: &[FoldRegion]) -> Option<usize> {
        let row = self.cursor_row()?;
        match self.visible_rows().get(row)? {
            VisibleRow::Fold { region_index, .. } => Some(*region_index),
            VisibleRow::Source { line } => regions
                .iter()
                .enumerate()
                .filter(|(_, region)| {
                    region.start_line <= *line
                        && *line <= region.end_line
                        && (!filters_dropped_declarations(self.loaded_mode) || region.retained)
                })
                .max_by_key(|(_, region)| region.level)
                .map(|(index, _)| index),
        }
    }

    pub(crate) fn toggle_marker_region(&mut self, row: usize, cx: &Cx) {
        let selected = cx.workspace.selected.clone();
        let selected = selected.as_ref();
        let Some(path) = selected else {
            return;
        };
        let Some(regions) = self.folds.regions(path).map(<[FoldRegion]>::to_vec) else {
            return;
        };
        let target = match self.visible_rows().get(row) {
            Some(VisibleRow::Fold { region_index, .. }) => Some(*region_index),
            Some(VisibleRow::Source { line }) => regions
                .iter()
                .enumerate()
                .filter(|(_, region)| {
                    region.start_line == *line
                        && (!filters_dropped_declarations(self.loaded_mode) || region.retained)
                })
                .max_by_key(|(_, region)| region.end_line)
                .map(|(index, _)| index),
            None => None,
        };
        let Some(target) = target else {
            return;
        };
        let open = !self.fold_state.is_open(path, &regions[target]);
        self.fold_state.set(path, &regions[target].stable_key, open);
        self.ensure_visible_rows(Some(path));
        self.reanchor_cursor(Some(path));
        self.clamp_view(cx.size.height, Some(path));
    }
}

fn subtree_keys(regions: &[FoldRegion], root: usize) -> Vec<String> {
    let root_key = regions[root].stable_key.as_str();
    let mut keys = vec![root_key.to_owned()];
    for region in regions {
        if region.stable_key == root_key {
            continue;
        }
        let mut current = region.parent_key.as_deref();
        let mut walked = 0;
        while let Some(parent) = current {
            if parent == root_key {
                keys.push(region.stable_key.clone());
                break;
            }
            walked += 1;
            if walked > regions.len() {
                break;
            }
            current = regions
                .iter()
                .find(|candidate| candidate.stable_key == parent)
                .and_then(|candidate| candidate.parent_key.as_deref());
        }
    }
    keys
}
