//! The source view's visible rows: source lines and folded summaries.

use std::collections::HashMap;
use std::sync::Arc;

use base::{FoldRegion, ProjectionMode, RepoPath, SourceOutline};

use tui_core::nav::scroll_offset_to;
use tui_core::render::highlight::{self, Run};
use tui_core::render::theme::Theme;
use tui_core::search::{SearchMatch, SearchPane, collect_matches};
use tui_core::workspace::{Workspace, mode_label};

use super::SourceView;
use super::folds::FoldState;

/// The body's scroll offsets and the source line the cursor sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShowBody {
    pub scroll: u16,
    pub hscroll: u16,
    pub cursor_line: usize,
}

/// One visible row: a real source line or a folded region's summary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisibleRow {
    Source {
        line: usize,
    },
    Fold {
        line: usize,
        summary_line: usize,
        region_index: usize,
        hidden: usize,
        dropped: bool,
    },
}

impl VisibleRow {
    pub fn source_line(&self) -> usize {
        match self {
            Self::Source { line } => *line,
            Self::Fold { summary_line, .. } => *summary_line,
        }
    }

    fn covers(&self, source_line: usize) -> bool {
        match self {
            Self::Source { line } => *line == source_line,
            Self::Fold { line, hidden, .. } => *line <= source_line && source_line <= line + hidden,
        }
    }
}

/// The cached visible rows and the fold version they were built from.
#[derive(Clone, Debug, Default)]
pub(crate) struct VisibleRows {
    key: Option<(RepoPath, u64)>,
    rows: Arc<[VisibleRow]>,
}

impl VisibleRows {
    fn is_current(&self, path: &RepoPath, version: u64) -> bool {
        self.key
            .as_ref()
            .is_some_and(|(key, seen)| key == path && *seen == version)
    }

    pub(crate) fn rows(&self) -> &[VisibleRow] {
        &self.rows
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }
}

pub(crate) fn show_visible(files: &[SourceOutline]) -> Vec<RepoPath> {
    files.iter().map(|file| file.path().clone()).collect()
}

impl SourceView {
    pub fn active_file(&self, selected: Option<&RepoPath>) -> Option<&SourceOutline> {
        let path = selected?;
        self.files.iter().find(|file| file.path() == path)
    }

    pub fn active_source(&self, selected: Option<&RepoPath>) -> Option<&str> {
        self.active_file(selected).map(|file| file.source.as_str())
    }

    pub fn active_lines(&self, selected: Option<&RepoPath>) -> Option<&[highlight::StyledLine]> {
        let path = selected?;
        self.highlight.lines(path)
    }

    pub fn line_count(&self) -> usize {
        self.visible_rows.len()
    }

    pub fn source_line_count(&self, selected: Option<&RepoPath>) -> usize {
        self.active_source(selected)
            .map_or(0, |text| text.lines().count())
    }

    pub fn visible_rows(&self) -> &[VisibleRow] {
        self.visible_rows.rows()
    }

    pub fn fold_marker(&self, selected: Option<&RepoPath>, row: &VisibleRow) -> Option<char> {
        match row {
            VisibleRow::Fold { .. } => Some('+'),
            VisibleRow::Source { line } => {
                let path = selected?;
                let regions = self.folds.regions(path)?;
                let region = regions
                    .iter()
                    .filter(|region| {
                        region.start_line == *line
                            && (self.loaded_mode != ProjectionMode::Tests || region.retained)
                    })
                    .max_by_key(|region| region.end_line)?;
                Some(if self.fold_state.is_open(path, region) {
                    '−'
                } else {
                    '+'
                })
            }
        }
    }

    pub fn visible_row_for_source_line(&self, line: usize) -> Option<usize> {
        self.visible_rows
            .rows()
            .iter()
            .position(|row| row.covers(line))
    }

    pub fn max_line_width(&self, selected: Option<&RepoPath>) -> usize {
        let Some(text) = self.active_source(selected) else {
            return 0;
        };
        let lines: Vec<&str> = text.lines().collect();
        self.visible_rows
            .rows()
            .iter()
            .map(|row| {
                let source = lines
                    .get(row.source_line().saturating_sub(1))
                    .copied()
                    .unwrap_or("");
                let width = unicode_width::UnicodeWidthStr::width(source);
                match row {
                    VisibleRow::Source { .. } => width,
                    VisibleRow::Fold {
                        hidden, dropped, ..
                    } => {
                        width
                            + unicode_width::UnicodeWidthStr::width(
                                fold_suffix(*hidden, *dropped, self.loaded_mode).as_str(),
                            )
                    }
                }
            })
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn ensure_visible_rows(&mut self, selected: Option<&RepoPath>) {
        let Some(path) = selected.cloned() else {
            self.visible_rows = VisibleRows::default();
            return;
        };
        let version = self.fold_state.version();
        if !self.visible_rows.is_current(&path, version) {
            self.ensure_folds(Some(&path));
            let total = self.source_line_count(Some(&path));
            let rows: Vec<VisibleRow> = match self.folds.regions(&path) {
                Some(regions) => {
                    build_visible_rows(regions, total, &path, &self.fold_state, self.loaded_mode)
                }
                None => (1..=total)
                    .map(|line| VisibleRow::Source { line })
                    .collect(),
            };
            self.visible_rows = VisibleRows {
                key: Some((path, version)),
                rows: Arc::from(rows),
            };
        }
        self.anchor_cursor_to_visible_row();
    }

    fn anchor_cursor_to_visible_row(&mut self) {
        if self.visible_rows.rows().is_empty() {
            self.body.cursor_line = 1;
            return;
        }
        if self
            .visible_row_for_source_line(self.body.cursor_line)
            .is_some()
        {
            return;
        }
        self.body.cursor_line = self
            .nearest_visible_line(self.body.cursor_line)
            .unwrap_or(1);
    }

    pub(crate) fn row_runs(
        &self,
        selected: Option<&RepoPath>,
        line: usize,
        theme: &Theme,
    ) -> Vec<Run> {
        if let Some(styled) = self.active_lines(selected) {
            return styled
                .get(line.saturating_sub(1))
                .cloned()
                .unwrap_or_default();
        }
        let text = self
            .active_source(selected)
            .and_then(|source| source.lines().nth(line.saturating_sub(1)))
            .unwrap_or("");
        vec![Run {
            style: theme.fg(theme.palette.text),
            text: text.to_owned(),
        }]
    }

    pub fn cursor_row(&self) -> Option<usize> {
        if self.visible_rows.rows().is_empty() {
            return None;
        }
        self.visible_row_for_source_line(self.body.cursor_line)
    }

    pub fn move_cursor_to_row(&mut self, index: usize, height: u16) {
        let count = self.line_count();
        if count == 0 {
            self.body.cursor_line = 1;
            return;
        }
        let index = index.min(count - 1);
        self.body.cursor_line = self.visible_rows.rows()[index].source_line();
        self.scroll_to_row(index, height);
    }

    pub(crate) fn reanchor_cursor(&mut self, selected: Option<&RepoPath>) {
        let total = self.source_line_count(selected);
        if total == 0 {
            self.body.cursor_line = 1;
            return;
        }
        let clamped = self.body.cursor_line.clamp(1, total);
        self.body.cursor_line = match self.visible_row_for_source_line(clamped) {
            Some(row) => self.visible_rows.rows()[row].source_line(),
            None => self.nearest_visible_line(clamped).unwrap_or(clamped),
        };
    }

    fn nearest_visible_line(&self, line: usize) -> Option<usize> {
        self.visible_rows
            .rows()
            .iter()
            .find(|row| row.source_line() >= line)
            .or_else(|| self.visible_rows.rows().last())
            .map(VisibleRow::source_line)
    }

    pub(crate) fn clamp_view(&mut self, height: u16, selected: Option<&RepoPath>) {
        let max = self.line_count().saturating_sub(1) as u16;
        self.body.scroll = self.body.scroll.min(max);
        let max_h = self.max_line_width(selected).saturating_sub(1) as u16;
        self.body.hscroll = self.body.hscroll.min(max_h);
        if let Some(row) = self.cursor_row() {
            self.scroll_to_row(row, height);
        }
    }

    pub(crate) fn reveal_source_line(&mut self, selected: Option<&RepoPath>, line: usize) {
        let Some(path) = selected else {
            return;
        };
        self.ensure_folds(Some(path));
        let Some(regions) = self.folds.regions(path).map(<[FoldRegion]>::to_vec) else {
            return;
        };
        let mut opened = false;
        for region in &regions {
            if region.start_line <= line
                && line <= region.end_line
                && !self.fold_state.is_open(path, region)
            {
                self.fold_state.set(path, &region.stable_key, true);
                opened = true;
            }
        }
        if opened {
            self.ensure_visible_rows(Some(path));
        }
    }

    pub(crate) fn move_cursor_to_source_line(&mut self, line: usize, height: u16) {
        if let Some(row) = self.visible_row_for_source_line(line) {
            self.move_cursor_to_row(row, height);
        }
    }

    fn scroll_to_row(&mut self, index: usize, height: u16) {
        let max = self.line_count().saturating_sub(1) as u16;
        if let Some(next) = scroll_offset_to(index, self.body.scroll as usize, viewport(height)) {
            self.body.scroll = next.min(max);
        }
    }

    /// The search matches for `needle` over the active file's source, keeping
    /// only the matches on a visible row.
    pub fn search_matches(&self, needle: &str, workspace: &Workspace) -> Vec<SearchMatch> {
        let needle_lower = needle.to_lowercase();
        let mut matches = Vec::new();
        if let Some(file) = self.active_file(workspace.selected.as_ref()) {
            collect_matches(
                &file.source,
                &needle_lower,
                SearchPane::Source,
                &mut matches,
            );
        }
        matches.retain(|matched| self.visible_row_for_source_line(matched.line).is_some());
        matches
    }
}

fn build_visible_rows(
    regions: &[FoldRegion],
    total: usize,
    path: &RepoPath,
    state: &FoldState,
    mode: ProjectionMode,
) -> Vec<VisibleRow> {
    if mode == ProjectionMode::Tests {
        build_filtered_rows(regions, total, path, state)
    } else {
        build_marker_rows(regions, total, path, state)
    }
}

fn build_marker_rows(
    regions: &[FoldRegion],
    total: usize,
    path: &RepoPath,
    state: &FoldState,
) -> Vec<VisibleRow> {
    let mut collapsed_at: HashMap<usize, usize> = HashMap::new();
    for (index, region) in regions.iter().enumerate() {
        if !state.is_open(path, region) {
            collapsed_at.entry(region.start_line).or_insert(index);
        }
    }

    let mut rows = Vec::with_capacity(total);
    let mut line = 1;
    while line <= total {
        match collapsed_at.get(&line) {
            Some(&index) => {
                let region = &regions[index];
                let end = region.end_line.clamp(line, total);
                rows.push(VisibleRow::Fold {
                    line,
                    summary_line: region.declaration_line.clamp(line, end),
                    region_index: index,
                    hidden: end - line,
                    dropped: !region.retained,
                });
                line = end + 1;
            }
            None => {
                rows.push(VisibleRow::Source { line });
                line += 1;
            }
        }
    }
    rows
}

fn build_filtered_rows(
    regions: &[FoldRegion],
    total: usize,
    path: &RepoPath,
    state: &FoldState,
) -> Vec<VisibleRow> {
    let mut coverage = vec![0i64; total + 2];
    for region in regions {
        if !region.retained || region.start_line > total {
            continue;
        }
        let start = region.start_line.max(1);
        let end = region.end_line.min(total);
        if start <= end {
            coverage[start] += 1;
            coverage[end + 1] -= 1;
        }
    }
    let mut in_retained = vec![false; total + 1];
    let mut open_runs = 0i64;
    for line in 1..=total {
        open_runs += coverage[line];
        in_retained[line] = open_runs > 0;
    }

    let mut collapsed_at: HashMap<usize, usize> = HashMap::new();
    let mut excluded_at: HashMap<usize, usize> = HashMap::new();
    for (index, region) in regions.iter().enumerate() {
        if !region.retained {
            excluded_at.entry(region.start_line).or_insert(index);
        } else if !state.is_open(path, region) {
            collapsed_at.entry(region.start_line).or_insert(index);
        }
    }

    let mut rows = Vec::new();
    let mut line = 1;
    while line <= total {
        if let Some(&index) = excluded_at.get(&line) {
            line = regions[index].end_line.clamp(line, total) + 1;
            continue;
        }
        if let Some(&index) = collapsed_at.get(&line) {
            let region = &regions[index];
            let end = region.end_line.clamp(line, total);
            rows.push(VisibleRow::Fold {
                line,
                summary_line: region.declaration_line.clamp(line, end),
                region_index: index,
                hidden: end - line,
                dropped: false,
            });
            line = end + 1;
            continue;
        }
        if in_retained[line] {
            rows.push(VisibleRow::Source { line });
        }
        line += 1;
    }
    rows
}

pub(crate) fn fold_suffix(hidden: usize, dropped: bool, mode: ProjectionMode) -> String {
    let mut suffix = String::from("  ⋯");
    if hidden > 0 {
        suffix.push_str(&format!(" {hidden} lines"));
    }
    if dropped {
        suffix.push_str(&format!(" hidden in {}", mode_label(mode)));
    }
    suffix
}

pub(crate) fn viewport(height: u16) -> usize {
    height.saturating_sub(4) as usize
}
