//! The `show` page: one revision's projection, the tree selection over it, the
//! body's scroll offsets, the committed search, and per-file highlighting.

use std::sync::Arc;

use base::{ProjectedFile, ProjectionMode, RepoPath, Selection};
use engine::EngineError;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::action::GlobalAction;
use crate::api::Cmd;
use crate::component::text_input::TextInput;
use crate::render::block::pane_block;
use crate::render::empty::render_empty;
use crate::render::highlight::{self, StyledLine};
use crate::render::layout::Edge;
use crate::render::text::clip_line;
use crate::render::theme::Theme;
use crate::route::ShowRequest;
use crate::util::cache::BoundedCache;
use crate::util::input::Key;

use super::{
    Ctx, OutMsg, Paging, Scope, Search, SearchSide, ViewCtx, advance_search, collect_matches,
    current_match, page_step, scroll_by, scroll_offset_to, search_ranges, text_edit,
};

/// The panes `Tab` cycles in a `show`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowFocus {
    Tree,
    Body,
}

/// The body scroll offsets of a `show`: vertical and horizontal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShowBody {
    pub scroll: u16,
    pub hscroll: u16,
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

/// A loaded `show` projection.
#[derive(Clone, Debug)]
pub struct Show {
    pub revision: String,
    pub mode: ProjectionMode,
    pub scope: Scope,
    pub files: Arc<[ProjectedFile]>,
    /// Paths the tree shows, in raw path-byte order.
    pub visible: Arc<[RepoPath]>,
    pub selected: Option<RepoPath>,
    pub focus: ShowFocus,
    pub body: ShowBody,
    pub paging: Paging,
    pub search: Option<Search>,
    pub highlight: ShowHighlights,
    /// The revision editor, which only exists on a `show`.
    pub prompt: Option<TextInput>,
}

impl Show {
    /// The projection currently shown by the body, if a file is selected.
    pub fn active_file(&self) -> Option<&ProjectedFile> {
        let path = self.selected.as_ref()?;
        self.files.iter().find(|file| file.path() == path)
    }

    pub fn active_text(&self) -> Option<&str> {
        self.active_file().map(ProjectedFile::canonical_text)
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

    pub fn line_count(&self) -> usize {
        self.active_text().map_or(0, |text| text.lines().count())
    }

    pub fn max_line_width(&self) -> usize {
        self.active_text().map_or(0, |text| {
            text.lines()
                .map(unicode_width::UnicodeWidthStr::width)
                .max()
                .unwrap_or(0)
        })
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
            collect_matches(
                file.canonical_text(),
                &needle_lower,
                SearchSide::Show,
                &mut matches,
            );
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
        let lines = highlight::highlight(file.canonical_text(), file.language(), theme);
        self.highlight.insert(path, lines);
    }
}

/// Builds a show scope's visible paths from its files.
pub fn show_visible(files: &[ProjectedFile]) -> Vec<RepoPath> {
    files
        .iter()
        .filter(|file| !file.canonical_text().is_empty())
        .map(|file| file.path().clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// A message the `show` page understands.
#[derive(Debug)]
pub enum Msg {
    /// A key that is not a semantic action, routed while the body has focus.
    Key(Key),
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
    Loaded(ShowRequest, Result<Arc<[ProjectedFile]>, Box<EngineError>>),
}

/// The pure `show` update. It mutates only the page and returns the shell work
/// it produced.
pub fn update(msg: Msg, page: &mut Show, ctx: &Ctx) -> Vec<OutMsg> {
    match msg {
        Msg::Key(key) => body_key(page, key),
        Msg::Scroll(delta) => {
            scroll_body(page, delta);
            Vec::new()
        }
        Msg::Action(action) => {
            let step = page_step(ctx.height);
            let max = page.line_count().saturating_sub(1) as u16;
            page.body.scroll = scroll_by(action, page.body.scroll, step, max);
            Vec::new()
        }
        Msg::StepSearch(forward) => {
            if let Some(matched) = advance_search(&mut page.search, forward) {
                let max = page.line_count().saturating_sub(1);
                let index = matched.line.saturating_sub(1);
                if let Some(next) = scroll_offset_to(
                    index,
                    page.body.scroll as usize,
                    ctx.height.saturating_sub(4) as usize,
                ) {
                    page.body.scroll = next.min(max as u16);
                }
            }
            Vec::new()
        }
        Msg::ScrollToCurrent => {
            if let Some(matched) = current_match(&page.search) {
                let max = page.line_count().saturating_sub(1);
                let index = matched.line.saturating_sub(1);
                if let Some(next) = scroll_offset_to(
                    index,
                    page.body.scroll as usize,
                    ctx.height.saturating_sub(4) as usize,
                ) {
                    page.body.scroll = next.min(max as u16);
                }
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

fn body_key(page: &mut Show, key: Key) -> Vec<OutMsg> {
    let max = page.line_count().saturating_sub(1) as u16;
    let max_h = page.max_line_width().saturating_sub(1) as u16;
    match key {
        Key::Down | Key::Char('j') => {
            page.body.scroll = page.body.scroll.saturating_add(1).min(max);
        }
        Key::Up | Key::Char('k') => {
            page.body.scroll = page.body.scroll.saturating_sub(1);
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
    result: Result<Arc<[ProjectedFile]>, Box<EngineError>>,
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
    let block = pane_block(" Projection ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(text) = page.active_text() else {
        let (title, detail) = if ctx.busy && ctx.tree_empty {
            ("Loading", "Projecting files…")
        } else if ctx.tree_empty {
            ("No files", "No projected file content in this scope.")
        } else {
            ("No file selected", "Select a file to view its projection.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    };

    let total = page.line_count();
    let height = inner.height as usize;
    let needs_scrollbar = total > height;
    let gutter = gutter_width(total);
    let width = (inner.width as usize).saturating_sub(gutter + usize::from(needs_scrollbar));
    let skip = page.body.scroll as usize;
    let hscroll = page.body.hscroll as usize;

    let mut lines = Vec::new();
    match page.active_lines() {
        Some(styled) => {
            for (offset, runs) in styled.iter().skip(skip).take(height).enumerate() {
                let number = skip + offset + 1;
                let ranges = page.search_ranges(number);
                let runs = highlight_search(runs, &ranges, theme);
                let clipped = highlight::clip_runs(&runs, hscroll, width);
                let mut spans = vec![gutter_span(number, gutter, theme)];
                spans.extend(
                    clipped
                        .into_iter()
                        .map(|run| Span::styled(run.text, run.style)),
                );
                lines.push(Line::from(spans));
            }
        }
        None => {
            for (offset, line) in text.lines().skip(skip).take(height).enumerate() {
                let number = skip + offset + 1;
                lines.push(Line::from(vec![
                    gutter_span(number, gutter, theme),
                    Span::styled(
                        clip_line(line, hscroll, width),
                        theme.fg(theme.palette.text),
                    ),
                ]));
            }
        }
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
