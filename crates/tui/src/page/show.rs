//! The `show` page: one revision's source explorer, the tree selection over it,
//! the body's scroll offsets, the committed search, and per-file highlighting.
//!
//! The body cursor always names a visible row's source line, so the `tests`-mode
//! filter (which can hide the cursor's line) anchors it to the nearest visible
//! row; both body offsets are clamped to the projection before the cursor is
//! revealed.
//!
//! In `tests` mode a dropped region's whole subtree is skipped. That is safe
//! because a container that holds a test is itself retained, so a dropped
//! container can have no retained descendant.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base::{FoldRegion, ProjectionMode, RepoPath, Selection, SourceOutline, fold_regions};
use engine::EngineError;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::action::{GlobalAction, ShowAction};
use crate::api::Cmd;
use crate::component::text_input::TextInput;
use crate::render::block::pane_block;
use crate::render::empty::render_empty;
use crate::render::highlight::{self, StyledLine};
use crate::render::layout::Edge;
use crate::render::theme::Theme;
use crate::route::ShowRequest;
use crate::util::cache::BoundedCache;
use crate::util::input::Key;

use super::{
    Ctx, OutMsg, Paging, Scope, Search, SearchSide, ViewCtx, advance_search, collect_matches,
    current_match, mode_label, page_step, scroll_offset_to, search_ranges, text_edit,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowFocus {
    Tree,
    Body,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShowBody {
    pub scroll: u16,
    pub hscroll: u16,
    pub cursor_line: usize,
}

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

    pub fn insert(&mut self, path: RepoPath, lines: Vec<StyledLine>) {
        self.cache.insert(path, Arc::new(lines));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

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

    pub fn insert(&mut self, path: RepoPath, regions: Vec<FoldRegion>) {
        self.cache.insert(path, Arc::new(regions));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

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

    fn version(&self) -> u64 {
        self.version
    }
}

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
}

#[derive(Clone, Debug)]
pub struct Show {
    pub revision: String,
    pub mode: ProjectionMode,
    pub scope: Scope,
    pub files: Arc<[SourceOutline]>,
    pub visible: Arc<[RepoPath]>,
    pub selected: Option<RepoPath>,
    pub focus: ShowFocus,
    pub body: ShowBody,
    pub paging: Paging,
    pub search: Option<Search>,
    pub highlight: ShowHighlights,
    pub folds: ShowFolds,
    pub fold_state: FoldState,
    pub(crate) visible_rows: VisibleRows,
    pub prompt: Option<TextInput>,
}

impl Show {
    pub fn active_file(&self) -> Option<&SourceOutline> {
        let path = self.selected.as_ref()?;
        self.files.iter().find(|file| file.path() == path)
    }

    pub fn active_source(&self) -> Option<&str> {
        self.active_file().map(|file| file.source.as_str())
    }

    pub fn active_lines(&self) -> Option<&[StyledLine]> {
        let path = self.selected.as_ref()?;
        self.highlight.lines(path)
    }

    pub fn search_ranges(&self, line: usize) -> Vec<(usize, usize, bool)> {
        search_ranges(self.search.as_ref(), SearchSide::Show, line)
    }

    pub fn line_count(&self) -> usize {
        self.visible_rows.rows.len()
    }

    pub fn source_line_count(&self) -> usize {
        self.active_source().map_or(0, |text| text.lines().count())
    }

    pub fn visible_rows(&self) -> &[VisibleRow] {
        &self.visible_rows.rows
    }

    pub fn visible_row_for_source_line(&self, line: usize) -> Option<usize> {
        self.visible_rows
            .rows
            .iter()
            .position(|row| row.covers(line))
    }

    pub fn max_line_width(&self) -> usize {
        let Some(text) = self.active_source() else {
            return 0;
        };
        let lines: Vec<&str> = text.lines().collect();
        self.visible_rows
            .rows
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
                                fold_suffix(*hidden, *dropped, self.mode).as_str(),
                            )
                    }
                }
            })
            .max()
            .unwrap_or(0)
    }

    pub fn ensure_visible_rows(&mut self) {
        let Some(path) = self.selected.clone() else {
            self.visible_rows = VisibleRows::default();
            return;
        };
        let version = self.fold_state.version();
        if !self.visible_rows.is_current(&path, version) {
            self.ensure_folds();
            let total = self.source_line_count();
            let rows: Vec<VisibleRow> = match self.folds.regions(&path) {
                Some(regions) => {
                    build_visible_rows(regions, total, &path, &self.fold_state, self.mode)
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
        if self.visible_rows.rows.is_empty() {
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

    fn ensure_folds(&mut self) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        if self.folds.contains(&path) {
            return;
        }
        let Some(file) = self.files.iter().find(|file| file.path() == &path) else {
            return;
        };
        let lines: Vec<&str> = file.source.lines().collect();
        let regions = fold_regions(&file.outline, &lines);
        self.folds.insert(path, regions);
    }

    fn row_runs(&self, line: usize, theme: &Theme) -> Vec<highlight::Run> {
        if let Some(styled) = self.active_lines() {
            return styled
                .get(line.saturating_sub(1))
                .cloned()
                .unwrap_or_default();
        }
        let text = self
            .active_source()
            .and_then(|source| source.lines().nth(line.saturating_sub(1)))
            .unwrap_or("");
        vec![highlight::Run {
            style: theme.fg(theme.palette.text),
            text: text.to_owned(),
        }]
    }

    pub fn cursor_row(&self) -> Option<usize> {
        if self.visible_rows.rows.is_empty() {
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
        self.body.cursor_line = self.visible_rows.rows[index].source_line();
        self.scroll_to_row(index, height);
    }

    pub fn reanchor_cursor(&mut self) {
        let total = self.source_line_count();
        if total == 0 {
            self.body.cursor_line = 1;
            return;
        }
        let clamped = self.body.cursor_line.clamp(1, total);
        self.body.cursor_line = match self.visible_row_for_source_line(clamped) {
            Some(row) => self.visible_rows.rows[row].source_line(),
            None => self.nearest_visible_line(clamped).unwrap_or(clamped),
        };
    }

    fn nearest_visible_line(&self, line: usize) -> Option<usize> {
        self.visible_rows
            .rows
            .iter()
            .find(|row| row.source_line() >= line)
            .or_else(|| self.visible_rows.rows.last())
            .map(VisibleRow::source_line)
    }

    pub fn clamp_view(&mut self, height: u16) {
        let max = self.line_count().saturating_sub(1) as u16;
        self.body.scroll = self.body.scroll.min(max);
        let max_h = self.max_line_width().saturating_sub(1) as u16;
        self.body.hscroll = self.body.hscroll.min(max_h);
        if let Some(row) = self.cursor_row() {
            self.scroll_to_row(row, height);
        }
    }

    pub fn reveal_source_line(&mut self, line: usize) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        self.ensure_folds();
        let Some(regions) = self.folds.regions(&path).map(<[FoldRegion]>::to_vec) else {
            return;
        };
        let mut opened = false;
        for region in &regions {
            if region.start_line <= line
                && line <= region.end_line
                && !self.fold_state.is_open(&path, region)
            {
                self.fold_state.set(&path, &region.stable_key, true);
                opened = true;
            }
        }
        if opened {
            self.ensure_visible_rows();
        }
    }

    fn move_cursor_to_source_line(&mut self, line: usize, height: u16) {
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

    pub fn set_search(&mut self, needle: &str) {
        if needle.is_empty() {
            self.search = None;
            return;
        }
        self.ensure_visible_rows();
        self.search = Some(Search {
            needle: needle.to_owned(),
            matches: self.search_matches(needle),
            cursor: 0,
        });
    }

    pub fn resync_search(&mut self) {
        let Some(needle) = self.search.as_ref().map(|search| search.needle.clone()) else {
            return;
        };
        self.ensure_visible_rows();
        let matches = self.search_matches(&needle);
        if let Some(search) = &mut self.search {
            search.matches = matches;
            search.cursor = search.cursor.min(search.matches.len().saturating_sub(1));
        }
    }

    fn search_matches(&self, needle: &str) -> Vec<super::SearchMatch> {
        let needle_lower = needle.to_lowercase();
        let mut matches = Vec::new();
        if let Some(file) = self.active_file() {
            collect_matches(&file.source, &needle_lower, SearchSide::Show, &mut matches);
        }
        matches.retain(|matched| self.visible_row_for_source_line(matched.line).is_some());
        matches
    }

    pub fn ensure_highlight(&mut self, theme: &Theme) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        if self.highlight.contains(&path) {
            return;
        }
        let Some(file) = self.files.iter().find(|file| file.path() == &path) else {
            return;
        };
        let lines = highlight::highlight(&file.source, file.language(), theme);
        self.highlight.insert(path, lines);
    }
}

pub fn show_visible(files: &[SourceOutline]) -> Vec<RepoPath> {
    files.iter().map(|file| file.path().clone()).collect()
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

fn fold_suffix(hidden: usize, dropped: bool, mode: ProjectionMode) -> String {
    let mut suffix = String::from("  ⋯");
    if hidden > 0 {
        suffix.push_str(&format!(" {hidden} lines"));
    }
    if dropped {
        suffix.push_str(&format!(" hidden in {}", mode_label(mode)));
    }
    suffix
}

fn viewport(height: u16) -> usize {
    height.saturating_sub(4) as usize
}

#[derive(Debug)]
pub enum Msg {
    Key(Key),
    Fold(ShowAction),
    Scroll(i32),
    Action(GlobalAction),
    StepSearch(bool),
    ScrollToCurrent,
    CycleFocus(bool),
    Prompt(Key),
    EditRevision,
    SetSearch(String),
    ClearSearch,
    ReloadScope(Selection),
    SwitchMode(ProjectionMode),
    Loaded(ShowRequest, Result<Arc<[SourceOutline]>, Box<EngineError>>),
}

pub fn update(msg: Msg, page: &mut Show, ctx: &Ctx) -> Vec<OutMsg> {
    page.ensure_visible_rows();
    match msg {
        Msg::Key(key) => body_key(page, key, ctx),
        Msg::Fold(action) => {
            fold_action(page, action, ctx.height);
            Vec::new()
        }
        Msg::Scroll(delta) => {
            scroll_body(page, delta);
            Vec::new()
        }
        Msg::Action(action) => {
            cursor_action(page, action, ctx);
            Vec::new()
        }
        Msg::StepSearch(forward) => {
            if let Some(matched) = advance_search(&mut page.search, forward) {
                page.reveal_source_line(matched.line);
                page.move_cursor_to_source_line(matched.line, ctx.height);
            }
            Vec::new()
        }
        Msg::ScrollToCurrent => {
            if let Some(matched) = current_match(&page.search) {
                page.reveal_source_line(matched.line);
                page.move_cursor_to_source_line(matched.line, ctx.height);
            }
            Vec::new()
        }
        Msg::CycleFocus(forward) => {
            page.focus = if forward {
                ShowFocus::Body
            } else {
                ShowFocus::Tree
            };
            Vec::new()
        }
        Msg::Prompt(key) => prompt_key(page, key),
        Msg::EditRevision => {
            page.prompt = Some(TextInput::new(page.revision.clone()));
            Vec::new()
        }
        Msg::SetSearch(needle) => {
            page.set_search(&needle);
            Vec::new()
        }
        Msg::ClearSearch => {
            page.search = None;
            Vec::new()
        }
        Msg::ReloadScope(selection) => {
            page.paging = Paging::Loading;
            let mut request = page.request();
            request.selection = selection;
            vec![OutMsg::Effect(Cmd::Show(request))]
        }
        Msg::SwitchMode(mode) => {
            page.paging = Paging::Loading;
            let mut request = page.request();
            request.mode = mode;
            vec![OutMsg::Effect(Cmd::Show(request))]
        }
        Msg::Loaded(request, result) => loaded(page, request, result),
    }
}

impl Show {
    pub fn request(&self) -> ShowRequest {
        ShowRequest {
            revision: self.revision.clone(),
            mode: self.mode,
            selection: self.scope.selection.clone(),
        }
    }
}

fn body_key(page: &mut Show, key: Key, ctx: &Ctx) -> Vec<OutMsg> {
    let max_h = page.max_line_width().saturating_sub(1) as u16;
    match key {
        Key::Down | Key::Char('j') => {
            let row = page.cursor_row().map_or(0, |row| row + 1);
            page.move_cursor_to_row(row, ctx.height);
        }
        Key::Up | Key::Char('k') => {
            let row = page.cursor_row().map_or(0, |row| row.saturating_sub(1));
            page.move_cursor_to_row(row, ctx.height);
        }
        Key::Right | Key::Char('l') => {
            page.body.hscroll = page.body.hscroll.saturating_add(1).min(max_h);
        }
        Key::Left | Key::Char('h') => {
            page.body.hscroll = page.body.hscroll.saturating_sub(1);
        }
        _ => {}
    }
    Vec::new()
}

fn cursor_action(page: &mut Show, action: GlobalAction, ctx: &Ctx) {
    let step = page_step(ctx.height) as usize;
    let current = page.cursor_row().unwrap_or(0);
    let last = page.line_count().saturating_sub(1);
    let row = match action {
        GlobalAction::Top => 0,
        GlobalAction::Bottom => last,
        GlobalAction::PageUp => current.saturating_sub(step),
        _ => (current + step).min(last),
    };
    page.move_cursor_to_row(row, ctx.height);
}

fn fold_action(page: &mut Show, action: ShowAction, height: u16) {
    let Some(path) = page.selected.clone() else {
        return;
    };
    let Some(regions) = page.folds.regions(&path).map(<[FoldRegion]>::to_vec) else {
        return;
    };
    if regions.is_empty() {
        return;
    }
    match action {
        ShowAction::OpenAll => {
            for region in &regions {
                page.fold_state.set(&path, &region.stable_key, true);
            }
        }
        ShowAction::CloseAll => {
            for region in &regions {
                page.fold_state.set(&path, &region.stable_key, false);
            }
        }
        _ => {
            let Some(target) = target_region(page, &regions) else {
                return;
            };
            match action {
                ShowAction::ToggleFold => {
                    let open = !page.fold_state.is_open(&path, &regions[target]);
                    page.fold_state
                        .set(&path, &regions[target].stable_key, open);
                }
                ShowAction::OpenFold => {
                    page.fold_state
                        .set(&path, &regions[target].stable_key, true);
                }
                ShowAction::CloseFold => {
                    page.fold_state
                        .set(&path, &regions[target].stable_key, false);
                }
                ShowAction::OpenRecursive => {
                    for key in subtree_keys(&regions, target) {
                        page.fold_state.set(&path, &key, true);
                    }
                }
                ShowAction::CloseRecursive => {
                    for key in subtree_keys(&regions, target) {
                        page.fold_state.set(&path, &key, false);
                    }
                }
                ShowAction::EditRevision | ShowAction::OpenAll | ShowAction::CloseAll => {}
            }
        }
    }
    page.ensure_visible_rows();
    page.reanchor_cursor();
    page.clamp_view(height);
}

fn target_region(page: &Show, regions: &[FoldRegion]) -> Option<usize> {
    let row = page.cursor_row()?;
    match page.visible_rows().get(row)? {
        VisibleRow::Fold { region_index, .. } => Some(*region_index),
        VisibleRow::Source { line } => regions
            .iter()
            .enumerate()
            .filter(|(_, region)| {
                region.start_line <= *line
                    && *line <= region.end_line
                    && (page.mode != ProjectionMode::Tests || region.retained)
            })
            .max_by_key(|(_, region)| region.level)
            .map(|(index, _)| index),
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

fn scroll_body(page: &mut Show, delta: i32) {
    let max = page.line_count().saturating_sub(1) as u16;
    let scroll = page.body.scroll;
    let next = if delta < 0 {
        scroll.saturating_sub(delta.unsigned_abs() as u16)
    } else {
        scroll.saturating_add(delta as u16).min(max)
    };
    page.body.scroll = next;
}

fn prompt_key(page: &mut Show, key: Key) -> Vec<OutMsg> {
    let Some(mut input) = page.prompt.take() else {
        return Vec::new();
    };
    let mut reopen = true;
    let mut effect = None;
    match key {
        Key::Esc => reopen = false,
        Key::Enter => {
            reopen = false;
            let value = input.value();
            if !value.is_empty() {
                page.paging = Paging::Loading;
                effect = Some(Cmd::Show(ShowRequest {
                    revision: value,
                    mode: page.mode,
                    selection: page.scope.selection.clone(),
                }));
            }
        }
        key => {
            if let Some(edit) = text_edit(key) {
                input.edit(edit);
            }
        }
    }
    if reopen {
        page.prompt = Some(input);
    }
    effect.into_iter().map(OutMsg::Effect).collect()
}

fn loaded(
    page: &mut Show,
    request: ShowRequest,
    result: Result<Arc<[SourceOutline]>, Box<EngineError>>,
) -> Vec<OutMsg> {
    match result {
        Ok(files) => {
            page.paging = Paging::Ready;
            page.revision = request.revision;
            page.mode = request.mode;
            page.scope = Scope::new(request.selection);
            page.files = files;
            page.visible = Arc::from(show_visible(&page.files));
            page.highlight.clear();
            page.folds.clear();
            page.visible_rows = VisibleRows::default();
            page.prompt = None;
            Vec::new()
        }
        Err(error) => {
            page.paging = Paging::Ready;
            vec![OutMsg::Diagnose(error.to_string())]
        }
    }
}

pub(crate) fn view(
    page: &Show,
    ctx: &ViewCtx,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let theme = ctx.theme;
    let block = pane_block(" Source ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if page.active_source().is_none() {
        let (title, detail) = if ctx.busy && ctx.tree_empty {
            ("Loading", "Projecting files…")
        } else if ctx.tree_empty {
            ("No files", "No source files in this scope.")
        } else {
            ("No file selected", "Select a file to view its source.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    if page.line_count() == 0 {
        let (title, detail) = if page.mode == ProjectionMode::Tests {
            ("No test declarations", "Nothing retained in this mode.")
        } else {
            ("Empty file", "This file has no source lines.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    let total = page.line_count();
    let source_total = page.source_line_count();
    let height = inner.height as usize;
    let needs_scrollbar = total > height;
    let gutter = gutter_width(source_total);
    let width = (inner.width as usize).saturating_sub(gutter + usize::from(needs_scrollbar));
    let skip = page.body.scroll as usize;
    let hscroll = page.body.hscroll as usize;
    let cursor_row = if focused { page.cursor_row() } else { None };
    let cursor_bg = theme.color(theme.palette.surface_alt);

    let mut lines = Vec::new();
    for (index, row) in page
        .visible_rows()
        .iter()
        .enumerate()
        .skip(skip)
        .take(height)
    {
        let number = row.source_line();
        let ranges = page.search_ranges(number);
        let mut runs = page.row_runs(number, theme);
        runs = highlight_search(&runs, &ranges, theme);
        if let VisibleRow::Fold {
            hidden, dropped, ..
        } = row
        {
            runs.push(highlight::Run {
                style: theme.fg(theme.palette.text_dim),
                text: fold_suffix(*hidden, *dropped, page.mode),
            });
        }
        let mut clipped = highlight::clip_runs(&runs, hscroll, width);
        let mut gutter = gutter_span(number, gutter, theme);
        if cursor_row == Some(index) {
            for run in &mut clipped {
                if run.style.bg.is_none() {
                    run.style = run.style.bg(cursor_bg);
                }
            }
            if gutter.style.bg.is_none() {
                gutter.style = gutter.style.bg(cursor_bg);
            }
        }
        let mut spans = vec![gutter];
        spans.extend(
            clipped
                .into_iter()
                .map(|run| Span::styled(run.text, run.style)),
        );
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if needs_scrollbar {
        let mut state = ScrollbarState::new(total)
            .position(page.body.scroll as usize)
            .viewport_content_length(height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(theme.fg(theme.palette.border))
                .begin_symbol(None)
                .end_symbol(None),
            inner,
            &mut state,
        );
    }
}

pub(crate) fn prompt_view(page: &Show, theme: &Theme, frame: &mut Frame, area: Rect) {
    if let Some(input) = &page.prompt {
        crate::component::text_input::render_prompt(frame, area, "revision", input, theme);
    }
}

fn gutter_width(total: usize) -> usize {
    total.max(1).to_string().len() + 1
}

fn highlight_search(
    runs: &[highlight::Run],
    ranges: &[(usize, usize, bool)],
    theme: &Theme,
) -> Vec<highlight::Run> {
    let mut styled = runs.to_vec();
    for (start, end, current) in ranges {
        let color = if *current {
            theme.color(theme.palette.match_current_bg)
        } else {
            theme.color(theme.palette.match_bg)
        };
        styled = highlight::apply_emphasis(&styled, &[(*start, *end)], color);
    }
    styled
}

fn gutter_span(number: usize, gutter: usize, theme: &Theme) -> Span<'static> {
    let field = gutter.saturating_sub(1);
    Span::styled(format!("{number:>field$} "), theme.fg(theme.palette.gutter))
}
