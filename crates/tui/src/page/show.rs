//! The `show` page: one revision's source explorer, the tree selection over it,
//! the body's scroll offsets, the committed search, and per-file highlighting.

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

/// The panes `Tab` cycles in a `show`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowFocus {
    Tree,
    Body,
}

/// The body scroll offsets of a `show`: vertical and horizontal, plus the
/// 1-based source line the body cursor sits on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShowBody {
    pub scroll: u16,
    pub hscroll: u16,
    /// The 1-based real source line the cursor sits on. It always names a
    /// visible row's source line: a line inside a collapsed region resolves to
    /// that region's fold row.
    pub cursor_line: usize,
}

/// Bounded, per-path syntax highlighting for a `show`.
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

/// Bounded, per-path fold regions for a `show`.
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

/// The sticky fold open/closed overrides for a `show`, keyed by path and
/// `stable_key`.
///
/// The map is empty until the interactive piece writes it; `is_open` already
/// consults it, so a later toggle needs no change here. `version` advances on
/// every `set` so the visible-row projection can tell its cache is stale.
#[derive(Clone, Debug, Default)]
pub struct FoldState {
    overrides: BTreeMap<RepoPath, BTreeMap<String, bool>>,
    version: u64,
}

impl FoldState {
    /// Whether `region` is open: its sticky override when one exists, else its
    /// `default_open`.
    pub fn is_open(&self, path: &RepoPath, region: &FoldRegion) -> bool {
        self.overrides
            .get(path)
            .and_then(|regions| regions.get(region.stable_key.as_str()))
            .copied()
            .unwrap_or(region.default_open)
    }

    /// Records a sticky open/closed override for `stable_key` on `path`.
    ///
    /// Nothing writes an override yet; the interactive folding piece calls
    /// this. `allow(dead_code)` keeps the read-only seam checked meanwhile.
    #[allow(dead_code)]
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

/// One rendered row of a `show` body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisibleRow {
    /// A real source line, rendered verbatim.
    Source {
        /// 1-based real source line.
        line: usize,
    },
    /// A collapsed declaration: its first line plus a fold suffix.
    Fold {
        /// 1-based real source line the declaration starts on.
        line: usize,
        /// Index of the region in the file's fold-region list.
        region_index: usize,
        /// How many source lines the fold hides (`end_line - line`).
        hidden: usize,
        /// Whether the requested mode drops the declaration.
        dropped: bool,
    },
}

impl VisibleRow {
    /// The 1-based source line this row starts on; a fold row names its
    /// declaration's start line.
    pub fn source_line(&self) -> usize {
        match self {
            Self::Source { line } | Self::Fold { line, .. } => *line,
        }
    }

    /// The last source line this row covers; a fold covers its hidden body, so
    /// a match inside the body maps back to the fold row.
    fn source_end(&self) -> usize {
        match self {
            Self::Source { line } => *line,
            Self::Fold { line, hidden, .. } => line + hidden,
        }
    }
}

/// The visible-row projection and the key it was built for.
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

/// A loaded `show` projection.
#[derive(Clone, Debug)]
pub struct Show {
    pub revision: String,
    pub mode: ProjectionMode,
    pub scope: Scope,
    pub files: Arc<[SourceOutline]>,
    /// Paths the tree shows, in raw path-byte order.
    pub visible: Arc<[RepoPath]>,
    pub selected: Option<RepoPath>,
    pub focus: ShowFocus,
    pub body: ShowBody,
    pub paging: Paging,
    pub search: Option<Search>,
    pub highlight: ShowHighlights,
    /// Fold regions per path, computed lazily, mirroring `highlight`.
    pub folds: ShowFolds,
    /// Sticky fold open/closed overrides; empty until interactive folding.
    pub fold_state: FoldState,
    /// The cached visible-row projection for the selected file.
    pub(crate) visible_rows: VisibleRows,
    /// The revision editor, which only exists on a `show`.
    pub prompt: Option<TextInput>,
}

impl Show {
    /// The file currently shown by the body, if one is selected.
    pub fn active_file(&self) -> Option<&SourceOutline> {
        let path = self.selected.as_ref()?;
        self.files.iter().find(|file| file.path() == path)
    }

    /// The real, unmodified source of the selected file.
    pub fn active_source(&self) -> Option<&str> {
        self.active_file().map(|file| file.source.as_str())
    }

    /// The syntax-highlighted lines of the selected file, if computed.
    pub fn active_lines(&self) -> Option<&[StyledLine]> {
        let path = self.selected.as_ref()?;
        self.highlight.lines(path)
    }

    /// The search ranges on one line of the show body.
    pub fn search_ranges(&self, line: usize) -> Vec<(usize, usize, bool)> {
        search_ranges(self.search.as_ref(), SearchSide::Show, line)
    }

    /// The number of visible rows the body renders; a folded declaration is one
    /// row.
    pub fn line_count(&self) -> usize {
        self.visible_rows.rows.len()
    }

    /// The number of real source lines in the selected file.
    pub fn source_line_count(&self) -> usize {
        self.active_source().map_or(0, |text| text.lines().count())
    }

    /// The visible-row projection of the selected file.
    pub fn visible_rows(&self) -> &[VisibleRow] {
        &self.visible_rows.rows
    }

    /// The visible-row index a 1-based source line maps to, or `None` when the
    /// line is past the file. A line inside a collapsed declaration resolves to
    /// its fold row.
    pub fn visible_row_for_source_line(&self, line: usize) -> Option<usize> {
        self.visible_rows
            .rows
            .iter()
            .position(|row| row.source_end() >= line)
    }

    /// The widest visible row, so horizontal scroll reaches a fold suffix too.
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

    /// Recomputes the visible-row projection when the selected path or the fold
    /// state changed, and clears it when nothing is selected.
    pub fn ensure_visible_rows(&mut self) {
        let Some(path) = self.selected.clone() else {
            self.visible_rows = VisibleRows::default();
            return;
        };
        let version = self.fold_state.version();
        if self.visible_rows.is_current(&path, version) {
            return;
        }
        self.ensure_folds();
        let total = self.source_line_count();
        let rows: Vec<VisibleRow> = match self.folds.regions(&path) {
            Some(regions) => build_visible_rows(regions, total, &path, &self.fold_state),
            None => (1..=total)
                .map(|line| VisibleRow::Source { line })
                .collect(),
        };
        self.visible_rows = VisibleRows {
            key: Some((path, version)),
            rows: Arc::from(rows),
        };
    }

    /// Computes and caches fold regions for the selected file when missing.
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

    /// The highlighted runs of one real source line, falling back to plain text
    /// when highlighting has not run.
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

    /// The visible-row index the cursor sits on, or `None` when no file or no
    /// rows.
    pub fn cursor_row(&self) -> Option<usize> {
        if self.visible_rows.rows.is_empty() {
            return None;
        }
        self.visible_row_for_source_line(self.body.cursor_line)
    }

    /// The source line of the first visible row, or `1` when there is none.
    pub fn first_visible_line(&self) -> usize {
        self.visible_rows
            .rows
            .first()
            .map_or(1, VisibleRow::source_line)
    }

    /// Moves the cursor to a visible-row index, clamped to the projection, and
    /// scrolls it into view.
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

    /// Re-anchors the cursor after a fold change or reload: clamps it to a real
    /// source line and snaps a now-hidden line to the nearest visible row.
    pub fn reanchor_cursor(&mut self) {
        let total = self.source_line_count();
        if total == 0 {
            self.body.cursor_line = 1;
            return;
        }
        let clamped = self.body.cursor_line.clamp(1, total);
        self.body.cursor_line = match self.visible_row_for_source_line(clamped) {
            Some(row) => self.visible_rows.rows[row].source_line(),
            None => clamped,
        };
    }

    /// Opens every region that encloses `line` so the matched source is visible,
    /// then recomputes the visible rows.
    ///
    /// The auto-open writes ordinary sticky overrides, so an explicit `zc`/`zM`
    /// can close the region again. A region already open (by default or by an
    /// earlier override) is left alone, so no needless recompute runs.
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

    /// Scrolls the body so the row covering a source line is visible.
    fn scroll_to_source_line(&mut self, line: usize, height: u16) {
        if let Some(row) = self.visible_row_for_source_line(line) {
            self.scroll_to_row(row, height);
        }
    }

    /// Scrolls the body so the row at `index` is visible.
    fn scroll_to_row(&mut self, index: usize, height: u16) {
        let max = self.line_count().saturating_sub(1) as u16;
        if let Some(next) = scroll_offset_to(index, self.body.scroll as usize, viewport(height)) {
            self.body.scroll = next.min(max);
        }
    }

    /// Recomputes the committed search for the current selection.
    pub fn set_search(&mut self, needle: &str) {
        if needle.is_empty() {
            self.search = None;
            return;
        }
        self.search = Some(Search {
            needle: needle.to_owned(),
            matches: self.search_matches(needle),
            cursor: 0,
        });
    }

    /// Recomputes a committed search after the selection changed.
    pub fn resync_search(&mut self) {
        let Some(needle) = self.search.as_ref().map(|search| search.needle.clone()) else {
            return;
        };
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
        matches
    }

    /// Computes and caches highlighting for the selected file when missing.
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

/// Builds a show scope's visible paths from its files.
///
/// Every file the engine returned is visible, in the engine's order. The body
/// renders real source, so a file whose requested-mode projection is empty — one
/// of only imports or comments, say — is still explorable.
pub fn show_visible(files: &[SourceOutline]) -> Vec<RepoPath> {
    files.iter().map(|file| file.path().clone()).collect()
}

/// Builds the visible-row projection from a file's fold regions.
///
/// The walk is top-down and O(lines + regions): a collapsed region is emitted
/// once and the cursor jumps past its body, so a nested region is never visited
/// while an ancestor is collapsed. Region lines are clamped to `total`, so a
/// span that runs past the file hides only what exists and a single-line region
/// yields a fold row with `hidden == 0`.
fn build_visible_rows(
    regions: &[FoldRegion],
    total: usize,
    path: &RepoPath,
    state: &FoldState,
) -> Vec<VisibleRow> {
    let mut collapsed_at: HashMap<usize, usize> = HashMap::new();
    for (index, region) in regions.iter().enumerate() {
        if !state.is_open(path, region) {
            // Source order puts an ancestor before its descendants, so the
            // first entry at a line is the outermost collapsed region.
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

/// The dim suffix a folded row appends after its real source line: a fold
/// affordance, the hidden line count when there is one, and the projection mode
/// that dropped the body.
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

/// The body rows the cursor is kept within, matching the search scroll budget.
fn viewport(height: u16) -> usize {
    height.saturating_sub(4) as usize
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// A message the `show` page understands.
#[derive(Debug)]
pub enum Msg {
    /// A key that is not a semantic action, routed while the body has focus.
    Key(Key),
    /// A semantic fold action routed from a key prefix or the command palette.
    Fold(ShowAction),
    /// A wheel scroll of the body by a signed number of rows.
    Scroll(i32),
    /// A scroll action: top, bottom, page up, or page down.
    Action(GlobalAction),
    /// Advance the committed search to the next or previous match.
    StepSearch(bool),
    /// Scroll the current search match into view without advancing.
    ScrollToCurrent,
    /// Cycle the focused pane forward or backward.
    CycleFocus(bool),
    /// A key typed while the revision prompt is open.
    Prompt(Key),
    /// Open the revision prompt on the current revision.
    EditRevision,
    /// Replace the committed search.
    SetSearch(String),
    /// Drop the committed search.
    ClearSearch,
    /// Reload the same projection with a new scope.
    ReloadScope(Selection),
    /// Reload the same projection in another mode.
    SwitchMode(ProjectionMode),
    /// A `show` projection effect finished.
    Loaded(ShowRequest, Result<Arc<[SourceOutline]>, Box<EngineError>>),
}

/// The pure `show` update. It mutates only the page and returns the shell work
/// it produced.
pub fn update(msg: Msg, page: &mut Show, ctx: &Ctx) -> Vec<OutMsg> {
    page.ensure_visible_rows();
    match msg {
        Msg::Key(key) => body_key(page, key, ctx),
        Msg::Fold(action) => {
            fold_action(page, action);
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
                page.scroll_to_source_line(matched.line, ctx.height);
            }
            Vec::new()
        }
        Msg::ScrollToCurrent => {
            if let Some(matched) = current_match(&page.search) {
                page.reveal_source_line(matched.line);
                page.scroll_to_source_line(matched.line, ctx.height);
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
    /// The effect request that reloads the projection exactly as it is.
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

/// Moves the body cursor for a top/bottom/page action, keeping it visible.
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

/// Applies one semantic fold action to the region at the cursor.
fn fold_action(page: &mut Show, action: ShowAction) {
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
}

/// The region a fold action targets: the fold row under the cursor, otherwise
/// the innermost region whose span contains the cursor's source line.
fn target_region(page: &Show, regions: &[FoldRegion]) -> Option<usize> {
    let row = page.cursor_row()?;
    match page.visible_rows().get(row)? {
        VisibleRow::Fold { region_index, .. } => Some(*region_index),
        VisibleRow::Source { line } => regions
            .iter()
            .enumerate()
            .filter(|(_, region)| region.start_line <= *line && *line <= region.end_line)
            .max_by_key(|(_, region)| region.level)
            .map(|(index, _)| index),
    }
}

/// The stable key of a region and every region nested under it, following
/// `parent_key`.
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

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// Draws the projection body.
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

/// Draws the revision prompt when it is open.
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
