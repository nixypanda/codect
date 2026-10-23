//! The source view's visible rows: source lines and folded summaries.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use base::{FoldRegion, ProjectionMode, RepoPath, SourceOutline};

use tui_core::nav::scroll_offset_to;
use tui_core::render::highlight::{self, Run};
use tui_core::render::theme::Theme;
use tui_core::search::{SearchMatch, SearchPane, collect_matches};
use tui_core::workspace::{Workspace, mode_label};

use super::SourceView;
use super::filters_dropped_declarations;
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

    /// Whether this row geometrically covers `source_line`.
    ///
    /// This is not the filter test: a retained collapsed container's fold row
    /// covers dropped children too. Callers that need source-line visibility
    /// must consult [`VisibleRows::line_visible`] first.
    fn covers(&self, source_line: usize) -> bool {
        match self {
            Self::Source { line } => *line == source_line,
            Self::Fold { line, hidden, .. } => *line <= source_line && source_line <= line + hidden,
        }
    }
}

/// The cached visible rows and the line-level visibility mask they were built
/// from.
#[derive(Clone, Debug, Default)]
pub(crate) struct VisibleRows {
    key: Option<(RepoPath, u64, ProjectionMode)>,
    rows: Arc<[VisibleRow]>,
    line_visible: Arc<[bool]>,
}

impl VisibleRows {
    fn is_current(&self, path: &RepoPath, version: u64, mode: ProjectionMode) -> bool {
        self.key.as_ref().is_some_and(|(key, seen, seen_mode)| {
            key == path && *seen == version && *seen_mode == mode
        })
    }

    pub(crate) fn rows(&self) -> &[VisibleRow] {
        &self.rows
    }

    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    fn line_visible(&self, line: usize) -> bool {
        self.line_visible.get(line).copied().unwrap_or(false)
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
                            && (!filters_dropped_declarations(self.loaded_mode) || region.retained)
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

    /// The row that displays `line`, or `None` when the filter hides it.
    ///
    /// A dropped line inside a collapsed retained container is hidden even
    /// though that container's fold row geometrically covers it. A retained
    /// line inside a collapsed container still maps to the fold row.
    pub fn visible_row_for_source_line(&self, line: usize) -> Option<usize> {
        if !self.visible_rows.line_visible(line) {
            return None;
        }
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
        let mode = self.loaded_mode;
        if !self.visible_rows.is_current(&path, version, mode) {
            self.ensure_folds(Some(&path));
            let total = self.source_line_count(Some(&path));
            let (rows, line_visible) = match self.folds.regions(&path) {
                Some(regions) => build_visible_rows(regions, total, &path, &self.fold_state, mode),
                None => (
                    (1..=total)
                        .map(|line| VisibleRow::Source { line })
                        .collect(),
                    unfiltered_line_mask(total),
                ),
            };
            self.visible_rows = VisibleRows {
                key: Some((path, version, mode)),
                rows: Arc::from(rows),
                line_visible: Arc::from(line_visible),
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
        self.ensure_visible_rows(Some(path));
        if !self.visible_rows.line_visible(line) {
            return;
        }
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
    /// only the matches on a line the filter leaves visible.
    ///
    /// “Searchable” means `visible_row_for_source_line` returns a row, not
    /// merely that a fold row geometrically covers the line. Retained lines
    /// inside a collapsed container still match and resolve to that fold row;
    /// dropped lines inside it do not. Types shared start, declaration, and end
    /// lines are searchable because the filter shows them, and an emitted Fold
    /// row's own summary line is always searchable.
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
) -> (Vec<VisibleRow>, Vec<bool>) {
    match mode {
        ProjectionMode::Types => {
            let line_visible = types_line_mask(regions, total);
            let rows = build_masked_rows(regions, total, path, state, &line_visible);
            (rows, line_visible)
        }
        ProjectionMode::Tests => {
            let coverage = line_coverage(regions, total);
            let rows = build_tests_rows(regions, total, path, state, &coverage);
            let line_visible = tests_line_mask(&coverage, &rows, total);
            (rows, line_visible)
        }
        ProjectionMode::Signatures => (
            build_marker_rows(regions, total, path, state),
            unfiltered_line_mask(total),
        ),
    }
}

/// Retained and dropped line coverage plus the retained region boundaries and
/// declaration lines.
struct LineCoverage {
    retained: Vec<bool>,
    dropped: Vec<bool>,
    retained_start: HashSet<usize>,
    retained_declaration: HashSet<usize>,
    retained_end: HashSet<usize>,
}

fn line_coverage(regions: &[FoldRegion], total: usize) -> LineCoverage {
    let mut retained_delta = vec![0i64; total + 2];
    let mut dropped_delta = vec![0i64; total + 2];
    let mut retained_start = HashSet::new();
    let mut retained_declaration = HashSet::new();
    let mut retained_end = HashSet::new();

    for region in regions {
        let start = region.start_line.max(1);
        let end = region.end_line.min(total);
        if start > end {
            continue;
        }
        if region.retained {
            retained_delta[start] += 1;
            retained_delta[end + 1] -= 1;
            retained_start.insert(start);
            retained_end.insert(end);
            if (1..=total).contains(&region.declaration_line) {
                retained_declaration.insert(region.declaration_line);
            }
        } else {
            dropped_delta[start] += 1;
            dropped_delta[end + 1] -= 1;
        }
    }

    let mut retained = vec![false; total + 1];
    let mut dropped = vec![false; total + 1];
    let mut open_retained = 0i64;
    let mut open_dropped = 0i64;
    for line in 1..=total {
        open_retained += retained_delta[line];
        open_dropped += dropped_delta[line];
        retained[line] = open_retained > 0;
        dropped[line] = open_dropped > 0;
    }

    LineCoverage {
        retained,
        dropped,
        retained_start,
        retained_declaration,
        retained_end,
    }
}

/// The Types filter's line mask.
///
/// A line is visible when it is inside a retained declaration/container and not
/// inside a dropped one. Types additionally keeps retained region start,
/// declaration, and end lines even when a dropped region covers the same
/// physical line: a terminal row cannot hide only part of a line, and dropping a
/// shared line would erase a retained one-line declaration, a collapsed row's
/// summary signature, or a retained container's closing brace.
fn types_line_mask(regions: &[FoldRegion], total: usize) -> Vec<bool> {
    let coverage = line_coverage(regions, total);
    let mut line_visible = vec![false; total + 1];
    for (line, visible) in line_visible.iter_mut().enumerate().skip(1) {
        *visible = coverage.retained[line]
            && (!coverage.dropped[line]
                || coverage.retained_start.contains(&line)
                || coverage.retained_declaration.contains(&line)
                || coverage.retained_end.contains(&line));
    }
    line_visible
}

/// Derive the Tests line mask from the rows the old Tests builder emitted.
///
/// Tests keeps its historical row construction, including drop-wins on shared
/// start lines and the multi-line fall-through that can follow it. Source rows
/// are visible. Inside a retained collapsed Fold row, retained lines are visible
/// and resolve to that fold row, while lines covered by dropped declarations are
/// not searchable. The emitted Fold row's own summary line is always visible so
/// search and cursor mapping can resolve the row, even when a dropped region
/// covers that line. This fixes dropped-line search/cursor mapping without
/// changing Tests row output.
fn tests_line_mask(coverage: &LineCoverage, rows: &[VisibleRow], total: usize) -> Vec<bool> {
    let mut line_visible = vec![false; total + 1];
    for row in rows {
        match row {
            VisibleRow::Source { line } => {
                if *line <= total {
                    line_visible[*line] = true;
                }
            }
            VisibleRow::Fold { line, hidden, .. } => {
                let end = line.saturating_add(*hidden).min(total);
                for (candidate, visible) in line_visible
                    .iter_mut()
                    .enumerate()
                    .take(end + 1)
                    .skip(*line)
                {
                    *visible = coverage.retained[candidate] && !coverage.dropped[candidate];
                }
                let summary = row.source_line();
                if summary <= total {
                    line_visible[summary] = true;
                }
            }
        }
    }
    line_visible
}

fn unfiltered_line_mask(total: usize) -> Vec<bool> {
    let mut line_visible = vec![true; total + 1];
    if let Some(first) = line_visible.first_mut() {
        *first = false;
    }
    line_visible
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

/// The Types filtered row builder: emit every line the mask leaves visible,
/// collapsing retained regions the user closed.
fn build_masked_rows(
    regions: &[FoldRegion],
    total: usize,
    path: &RepoPath,
    state: &FoldState,
    line_visible: &[bool],
) -> Vec<VisibleRow> {
    let mut collapsed_at: HashMap<usize, usize> = HashMap::new();
    for (index, region) in regions.iter().enumerate() {
        if region.retained && !state.is_open(path, region) {
            collapsed_at.entry(region.start_line).or_insert(index);
        }
    }

    let mut rows = Vec::new();
    let mut line = 1;
    while line <= total {
        if !line_visible.get(line).copied().unwrap_or(false) {
            line += 1;
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
        rows.push(VisibleRow::Source { line });
        line += 1;
    }
    rows
}

/// The historical Tests row builder.
///
/// Tests row output is deliberately unchanged from main: the first dropped
/// region at a start line wins the whole line, retained collapsed regions emit
/// one Fold row, and `excluded_at` holds only the first dropped region per start
/// line. Do not replace this with the Types mask; the characterization tests
/// lock the shared-line behavior.
fn build_tests_rows(
    regions: &[FoldRegion],
    total: usize,
    path: &RepoPath,
    state: &FoldState,
    coverage: &LineCoverage,
) -> Vec<VisibleRow> {
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
        if coverage.retained.get(line).copied().unwrap_or(false) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use base::ItemKind;

    fn region(
        key: &str,
        retained: bool,
        start: usize,
        end: usize,
        level: usize,
        default_open: bool,
    ) -> FoldRegion {
        FoldRegion {
            stable_key: key.to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            retained,
            start_line: start,
            declaration_line: start,
            end_line: end,
            level,
            default_open,
        }
    }

    fn visible_lines(mask: &[bool]) -> Vec<usize> {
        mask.iter()
            .enumerate()
            .skip(1)
            .filter_map(|(line, visible)| visible.then_some(line))
            .collect()
    }

    fn build(
        regions: &[FoldRegion],
        total: usize,
        mode: ProjectionMode,
    ) -> (Vec<VisibleRow>, Vec<bool>) {
        let path = RepoPath::new("a.rs").expect("valid path");
        let state = FoldState::default();
        build_visible_rows(regions, total, &path, &state, mode)
    }

    #[test]
    fn types_line_mask_hides_dropped_and_unretained_lines() {
        let regions = [
            region("container", true, 1, 8, 1, true),
            region("dropped", false, 3, 4, 2, false),
            region("child", true, 6, 6, 2, true),
            region("leaf", false, 9, 9, 1, false),
        ];
        let mask = types_line_mask(&regions, 10);
        assert_eq!(visible_lines(&mask), vec![1, 2, 5, 6, 7, 8]);
    }

    #[test]
    fn types_line_mask_re_enables_retained_starts() {
        let regions = [
            region("container", true, 1, 1, 1, true),
            region("dropped", false, 1, 1, 2, false),
        ];
        let mask = types_line_mask(&regions, 1);
        assert_eq!(visible_lines(&mask), vec![1]);
    }

    #[test]
    fn types_line_mask_re_enables_retained_ends() {
        let regions = [
            region("container", true, 1, 3, 1, true),
            region("dropped", false, 1, 3, 2, false),
        ];
        let mask = types_line_mask(&regions, 3);
        assert_eq!(visible_lines(&mask), vec![1, 3]);
    }

    #[test]
    fn types_line_mask_re_enables_retained_declaration_lines() {
        let mut container = region("container", true, 1, 4, 1, true);
        container.declaration_line = 2;
        let mut dropped = region("dropped", false, 1, 4, 2, false);
        dropped.declaration_line = 2;
        let mask = types_line_mask(&[container, dropped], 4);
        assert_eq!(
            visible_lines(&mask),
            vec![1, 2, 4],
            "the collapsed-row summary signature stays visible"
        );
    }

    #[test]
    fn filtered_rows_preserve_tests_drop_wins_on_shared_start_lines() {
        let regions = [
            region("module", true, 1, 1, 1, true),
            region("test", true, 1, 1, 2, false),
            region("helper", false, 1, 1, 2, false),
        ];
        let (rows, mask) = build(&regions, 1, ProjectionMode::Tests);
        assert!(rows.is_empty());
        assert!(visible_lines(&mask).is_empty());
    }

    #[test]
    fn filtered_rows_keep_types_shared_start_lines() {
        let regions = [
            region("container", true, 1, 1, 1, true),
            region("dropped", false, 1, 1, 2, false),
        ];
        let (rows, _) = build(&regions, 1, ProjectionMode::Types);
        assert_eq!(rows, vec![VisibleRow::Source { line: 1 }]);
    }

    #[test]
    fn filtered_rows_keep_types_shared_end_lines() {
        let regions = [
            region("container", true, 1, 3, 1, true),
            region("dropped", false, 1, 3, 2, false),
        ];
        let (rows, _) = build(&regions, 3, ProjectionMode::Types);
        assert_eq!(
            rows,
            vec![
                VisibleRow::Source { line: 1 },
                VisibleRow::Source { line: 3 }
            ]
        );

        let (rows, _) = build(&regions, 3, ProjectionMode::Tests);
        assert!(rows.is_empty());
    }

    #[test]
    fn filtered_rows_characterize_tests_multiline_shared_start() {
        let regions = [
            region("module", true, 1, 3, 1, true),
            region("test", true, 1, 3, 2, false),
            region("helper", false, 1, 1, 2, false),
        ];
        let (rows, _) = build(&regions, 3, ProjectionMode::Tests);
        assert_eq!(
            rows,
            vec![
                VisibleRow::Source { line: 2 },
                VisibleRow::Source { line: 3 }
            ],
            "Tests keeps the pre-existing drop-wins quirk: the collapsed test row is not emitted, so later retained body lines fall through as source rows"
        );
    }

    #[test]
    fn filtered_rows_preserve_tests_rows_when_dropped_declarations_share_a_start_line() {
        // Two dropped declarations share start line 1. The historical Tests
        // builder stores only the first one in `excluded_at`, so it jumps to
        // line 2. That visible line is dropped-b's body, not the retained test;
        // the retained test at line 4 is not emitted in this historical row
        // shape. The test locks that exact row output and derived mask.
        let regions = [
            region("module", true, 1, 4, 1, true),
            region("test", true, 1, 4, 2, false),
            region("dropped-a", false, 1, 1, 3, false),
            region("dropped-b", false, 1, 3, 3, false),
            region("dropped-c", false, 3, 4, 3, false),
        ];
        let (rows, mask) = build(&regions, 4, ProjectionMode::Tests);
        assert_eq!(
            rows,
            vec![VisibleRow::Source { line: 2 }],
            "line 2 is dropped-b's body and stays visible exactly as the old Tests builder rendered it"
        );
        assert_eq!(visible_lines(&mask), vec![2]);
    }

    #[test]
    fn tests_line_mask_keeps_an_emitted_fold_summary_visible() {
        let regions = [
            region("module", true, 1, 3, 1, true),
            region("test", true, 3, 3, 2, false),
            region("dropped-a", false, 1, 1, 3, false),
            region("dropped-b", false, 1, 3, 3, false),
        ];
        let (rows, mask) = build(&regions, 3, ProjectionMode::Tests);
        assert_eq!(
            rows,
            vec![
                VisibleRow::Source { line: 2 },
                VisibleRow::Fold {
                    line: 3,
                    summary_line: 3,
                    region_index: 1,
                    hidden: 0,
                    dropped: false,
                },
            ],
            "the old Tests builder emits the retained test as a Fold summary"
        );
        assert_eq!(
            visible_lines(&mask),
            vec![2, 3],
            "the emitted Fold row's own summary line must stay visible/searchable"
        );
    }

    #[test]
    fn filtered_rows_emit_fold_rows_only_for_retained_collapsed_regions() {
        let regions = [
            region("container", true, 1, 5, 1, false),
            region("dropped", false, 2, 3, 2, false),
        ];
        let (rows, _) = build(&regions, 5, ProjectionMode::Types);
        assert_eq!(
            rows,
            vec![VisibleRow::Fold {
                line: 1,
                summary_line: 1,
                region_index: 0,
                hidden: 4,
                dropped: false,
            }]
        );
    }

    #[test]
    fn filtered_rows_hide_single_line_declarations_without_a_newline() {
        let dropped = [region("dropped", false, 1, 1, 1, false)];
        let (rows, _) = build(&dropped, 1, ProjectionMode::Types);
        assert!(rows.is_empty());

        let retained = [region("retained", true, 1, 1, 1, true)];
        let (rows, _) = build(&retained, 1, ProjectionMode::Types);
        assert_eq!(rows, vec![VisibleRow::Source { line: 1 }]);
    }

    #[test]
    fn filtered_rows_hide_a_file_with_no_outline() {
        let (rows, mask) = build(&[], 3, ProjectionMode::Types);
        assert!(rows.is_empty());
        assert!(visible_lines(&mask).is_empty());
    }
}
