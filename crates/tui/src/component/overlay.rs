//! The modal overlays that are valid whatever kind of projection is loaded:
//! help, the scope chooser, the mode picker, the command palette, the fuzzy
//! file finder, and search.
//!
//! The revision editors are *not* here: a base-revision prompt only exists on a
//! diff and a show-revision prompt only on a show, so each lives in its content
//! variant.

use base::{AreaSet, ProjectionMode, RepoPath, Selection, SelectionGroup};
use engine::EngineError;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::action::Action;
use crate::page::{available_modes, mode_label};
use crate::render::block::{popup_block, scrim};
use crate::render::layout::{centered, window_offset};
use crate::render::theme::Theme;
use crate::util::fuzzy;
use crate::util::input::Key;

use super::text_input::{Edit, TextInput, input_line};

/// One ranked palette or finder result: an index into the source list, a fuzzy
/// score, and the byte offsets that matched for highlighting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ranked {
    pub index: usize,
    pub score: i32,
    pub positions: Vec<usize>,
}

/// The command palette: a fuzzy list of actions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaletteState {
    pub input: TextInput,
    pub matches: Vec<Ranked>,
    pub cursor: usize,
}

/// The fuzzy file finder: a fuzzy list of visible paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinderState {
    pub input: TextInput,
    pub matches: Vec<Ranked>,
    pub cursor: usize,
}

/// The search input. Live matches are written straight to the loaded search so
/// the view previews them while the user types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchState {
    pub input: TextInput,
}

/// The scope chooser's state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeChooser {
    /// `None` while areas are still loading.
    pub areas: Option<AreaSet>,
    pub cursor: usize,
    /// `Some` while the user is typing a literal path.
    pub input: Option<TextInput>,
    /// A chooser-local failure, such as a malformed config or bad path.
    pub error: Option<String>,
}

impl ScopeChooser {
    fn loading() -> Self {
        Self {
            areas: None,
            cursor: 0,
            input: None,
            error: None,
        }
    }

    /// `all`, each area name, then the literal-path entry.
    pub fn options(&self) -> Vec<String> {
        let mut options = vec!["all".to_owned()];
        if let Some(areas) = &self.areas {
            options.extend(areas.names().map(str::to_owned));
        }
        options.push("path…".to_owned());
        options
    }

    fn area_count(&self) -> usize {
        self.areas.as_ref().map_or(0, |areas| areas.names().count())
    }
}

/// A modal interaction that captures keys until it closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    Help,
    Scope(ScopeChooser),
    Mode { cursor: usize },
    Palette(PaletteState),
    Finder(FinderState),
    Search(SearchState),
}

/// What the parent offers the overlay so it can refresh its fuzzy lists.
pub struct Ctx<'a> {
    pub visible: &'a [RepoPath],
    pub entries: &'a [(Action, &'static str, &'static str)],
    pub mode: ProjectionMode,
}

/// A message the overlay understands.
#[derive(Debug)]
pub enum Msg {
    Key(Key),
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

/// What the overlay asks its parent to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutMsg {
    /// Close the overlay.
    Close,
    /// A scope was chosen.
    Selection(Selection),
    /// A projection mode was chosen.
    Mode(ProjectionMode),
    /// A palette entry was chosen.
    Action(Action),
    /// A finder result was chosen.
    File(RepoPath),
    /// The search input changed.
    Search(String),
    /// The search was committed.
    SearchCommit,
}

impl Overlay {
    pub fn help() -> Self {
        Self::Help
    }

    pub fn scope() -> Self {
        Self::Scope(ScopeChooser::loading())
    }

    pub fn mode(current: ProjectionMode) -> Self {
        let modes = available_modes();
        let cursor = modes.iter().position(|mode| *mode == current).unwrap_or(0);
        Self::Mode { cursor }
    }

    pub fn palette(ctx: &Ctx) -> Self {
        let mut state = PaletteState {
            input: TextInput::new(""),
            matches: Vec::new(),
            cursor: 0,
        };
        state.refresh_entries(ctx.entries);
        Self::Palette(state)
    }

    pub fn finder(ctx: &Ctx) -> Self {
        let mut state = FinderState {
            input: TextInput::new(""),
            matches: Vec::new(),
            cursor: 0,
        };
        state.refresh_visible(ctx.visible);
        Self::Finder(state)
    }

    pub fn search() -> Self {
        Self::Search(SearchState {
            input: TextInput::new(""),
        })
    }

    /// Whether the overlay captures text, so `q` types instead of quitting.
    pub fn captures_text(&self) -> bool {
        matches!(
            self,
            Self::Scope(_) | Self::Palette(_) | Self::Finder(_) | Self::Search(_)
        )
    }

    /// Applies a message, returning the parent work it produced.
    pub fn update(&mut self, msg: Msg, ctx: &Ctx) -> Vec<OutMsg> {
        match msg {
            Msg::AreasLoaded(result) => {
                if let Self::Scope(chooser) = self {
                    match result {
                        Ok(areas) => {
                            chooser.cursor = chooser.cursor.min(areas.names().count());
                            chooser.areas = Some(areas);
                            chooser.error = None;
                        }
                        Err(error) => chooser.error = Some(error.to_string()),
                    }
                }
                Vec::new()
            }
            Msg::Key(key) => self.key(key, ctx),
        }
    }

    fn key(&mut self, key: Key, ctx: &Ctx) -> Vec<OutMsg> {
        match self {
            Self::Help => {
                // A dismissal key closes help; every other key is swallowed.
                if key == Key::Esc || key == Key::Char('?') {
                    vec![OutMsg::Close]
                } else {
                    Vec::new()
                }
            }
            Self::Scope(chooser) => scope_key(chooser, key),
            Self::Mode { cursor } => mode_key(cursor, key, ctx.mode),
            Self::Palette(state) => palette_key(state, key, ctx.entries),
            Self::Finder(state) => finder_key(state, key, ctx.visible),
            Self::Search(state) => search_key(state, key),
        }
    }
}

fn scope_key(chooser: &mut ScopeChooser, key: Key) -> Vec<OutMsg> {
    match key {
        Key::Esc => {
            if chooser.input.is_some() {
                chooser.input = None;
                chooser.error = None;
                Vec::new()
            } else {
                vec![OutMsg::Close]
            }
        }
        Key::Enter if chooser.input.is_some() => {
            let Some(input) = chooser.input.take() else {
                return Vec::new();
            };
            match RepoPath::new(input.value()) {
                Ok(path) => vec![
                    OutMsg::Close,
                    OutMsg::Selection(Selection::new(vec![SelectionGroup::Path(path)])),
                ],
                Err(error) => {
                    chooser.error = Some(error.to_string());
                    chooser.input = Some(input);
                    Vec::new()
                }
            }
        }
        Key::Enter => {
            let area_count = chooser.area_count();
            let cursor = chooser.cursor.min(area_count + 1);
            if cursor == 0 {
                vec![OutMsg::Close, OutMsg::Selection(Selection::all())]
            } else if cursor <= area_count {
                let area = chooser.areas.as_ref().and_then(|areas| {
                    let name = areas.names().nth(cursor - 1)?;
                    areas.get(name)
                });
                match area.cloned() {
                    Some(area) => vec![
                        OutMsg::Close,
                        OutMsg::Selection(Selection::new(vec![SelectionGroup::Area(area)])),
                    ],
                    None => Vec::new(),
                }
            } else {
                chooser.input = Some(TextInput::new(""));
                Vec::new()
            }
        }
        Key::Up | Key::Char('k') if chooser.input.is_none() => {
            chooser.cursor = chooser.cursor.saturating_sub(1);
            Vec::new()
        }
        Key::Down | Key::Char('j') if chooser.input.is_none() => {
            let last = chooser.options().len().saturating_sub(1);
            chooser.cursor = (chooser.cursor + 1).min(last);
            Vec::new()
        }
        key if chooser.input.is_some() => {
            if let Some(edit) = text_edit(key)
                && let Some(input) = &mut chooser.input
            {
                input.edit(edit);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn mode_key(cursor: &mut usize, key: Key, _current: ProjectionMode) -> Vec<OutMsg> {
    let modes = available_modes();
    match key {
        Key::Esc => vec![OutMsg::Close],
        Key::Up | Key::Char('k') => {
            *cursor = cursor.saturating_sub(1);
            Vec::new()
        }
        Key::Down | Key::Char('j') => {
            *cursor = (*cursor + 1).min(modes.len() - 1);
            Vec::new()
        }
        Key::Enter => {
            let mode = modes[(*cursor).min(modes.len() - 1)];
            vec![OutMsg::Close, OutMsg::Mode(mode)]
        }
        _ => Vec::new(),
    }
}

fn palette_key(
    state: &mut PaletteState,
    key: Key,
    entries: &[(Action, &'static str, &'static str)],
) -> Vec<OutMsg> {
    match key {
        Key::Esc => vec![OutMsg::Close],
        Key::Up => {
            state.cursor = state.cursor.saturating_sub(1);
            Vec::new()
        }
        Key::Down => {
            state.cursor = (state.cursor + 1).min(state.matches.len().saturating_sub(1));
            Vec::new()
        }
        Key::Enter => {
            let action = state
                .matches
                .get(state.cursor)
                .and_then(|ranked| entries.get(ranked.index))
                .map(|(action, _, _)| *action);
            match action {
                Some(action) => vec![OutMsg::Close, OutMsg::Action(action)],
                None => vec![OutMsg::Close],
            }
        }
        Key::Char(character) => {
            state.input.edit(Edit::Insert(character));
            state.cursor = 0;
            state.refresh_entries(entries);
            Vec::new()
        }
        Key::Backspace => {
            state.input.edit(Edit::Backspace);
            state.cursor = 0;
            state.refresh_entries(entries);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn finder_key(state: &mut FinderState, key: Key, visible: &[RepoPath]) -> Vec<OutMsg> {
    match key {
        Key::Esc => vec![OutMsg::Close],
        Key::Up => {
            state.cursor = state.cursor.saturating_sub(1);
            Vec::new()
        }
        Key::Down => {
            state.cursor = (state.cursor + 1).min(state.matches.len().saturating_sub(1));
            Vec::new()
        }
        Key::Enter => {
            let path = state
                .matches
                .get(state.cursor)
                .and_then(|ranked| visible.get(ranked.index))
                .cloned();
            match path {
                Some(path) => vec![OutMsg::Close, OutMsg::File(path)],
                None => vec![OutMsg::Close],
            }
        }
        Key::Char(character) => {
            state.input.edit(Edit::Insert(character));
            state.cursor = 0;
            state.refresh_visible(visible);
            Vec::new()
        }
        Key::Backspace => {
            state.input.edit(Edit::Backspace);
            state.cursor = 0;
            state.refresh_visible(visible);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn search_key(state: &mut SearchState, key: Key) -> Vec<OutMsg> {
    match key {
        Key::Esc => vec![OutMsg::Close, OutMsg::Search(String::new())],
        Key::Enter => vec![OutMsg::Close, OutMsg::SearchCommit],
        Key::Char(character) => {
            state.input.edit(Edit::Insert(character));
            vec![OutMsg::Search(state.input.value())]
        }
        Key::Backspace => {
            state.input.edit(Edit::Backspace);
            vec![OutMsg::Search(state.input.value())]
        }
        _ => Vec::new(),
    }
}

fn text_edit(key: Key) -> Option<Edit> {
    match key {
        Key::Char(character) => Some(Edit::Insert(character)),
        Key::Backspace => Some(Edit::Backspace),
        Key::Delete => Some(Edit::Delete),
        Key::Left => Some(Edit::Left),
        Key::Right => Some(Edit::Right),
        Key::Home => Some(Edit::Home),
        Key::End => Some(Edit::End),
        _ => None,
    }
}

impl PaletteState {
    fn refresh_entries(&mut self, entries: &[(Action, &'static str, &'static str)]) {
        let needle = self.input.text.clone();
        let mut matches: Vec<Ranked> = entries
            .iter()
            .enumerate()
            .filter_map(|(index, (_, label, _))| {
                fuzzy::fuzzy(&needle, label).map(|matched| Ranked {
                    index,
                    score: matched.score,
                    positions: matched.positions,
                })
            })
            .collect();
        matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        self.matches = matches;
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }
}

impl FinderState {
    fn refresh_visible(&mut self, visible: &[RepoPath]) {
        let needle = self.input.text.clone();
        let mut matches: Vec<Ranked> = visible
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let text = path.to_string();
                fuzzy::fuzzy(&needle, &text).map(|matched| Ranked {
                    index,
                    score: matched.score,
                    positions: matched.positions,
                })
            })
            .collect();
        matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        matches.truncate(200);
        self.matches = matches;
        self.cursor = self.cursor.min(self.matches.len().saturating_sub(1));
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

/// What the overlay renderer reads from the app, beyond its own state.
pub(crate) struct RenderCtx<'a> {
    pub theme: &'a Theme,
    pub mode: ProjectionMode,
    pub entries: &'a [(Action, &'static str, &'static str)],
    pub visible: &'a [RepoPath],
    /// The committed search's match count, for the search summary line.
    pub matches: usize,
}

/// Draws the modal overlay. The revision prompts are *not* here: they belong to
/// the loaded projection and are drawn by [`crate::view::prompt`].
pub(crate) fn render(state: &Overlay, ctx: &RenderCtx, frame: &mut Frame, area: Rect) {
    let theme = ctx.theme;
    match state {
        Overlay::Help => render_help(frame, area, theme),
        Overlay::Scope(chooser) => render_scope(frame, area, chooser, theme),
        Overlay::Mode { cursor } => render_mode(frame, area, *cursor, ctx.mode, theme),
        Overlay::Palette(state) => render_palette(frame, area, state, ctx, theme),
        Overlay::Finder(state) => render_finder(frame, area, state, ctx, theme),
        Overlay::Search(state) => render_search(frame, area, state, ctx, theme),
    }
}

fn render_mode(
    frame: &mut Frame,
    area: Rect,
    cursor: usize,
    current: ProjectionMode,
    theme: &Theme,
) {
    let modes = available_modes();
    let width = area.width.saturating_sub(4).min(40);
    let height = (modes.len() + 3).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("mode", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = Vec::new();
    for (index, mode) in modes.iter().enumerate() {
        let selected = index == cursor;
        let style = if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
        let marker = if *mode == current { "•" } else { " " };
        lines.push(Line::from(Span::styled(
            format!(" {marker} {} ", mode_label(*mode)),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_scope(frame: &mut Frame, area: Rect, chooser: &ScopeChooser, theme: &Theme) {
    let options = chooser.options();
    let extra = 4 + usize::from(chooser.input.is_some()) * 2 + usize::from(chooser.error.is_some());
    let width = area.width.saturating_sub(4).min(60);
    let height = (options.len() + extra).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("scope", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = Vec::new();
    if chooser.areas.is_none() && chooser.error.is_none() {
        lines.push(Line::from(Span::styled(
            " loading areas…",
            theme.fg(theme.palette.text_muted),
        )));
    }
    for (index, option) in options.iter().enumerate() {
        let selected = index == chooser.cursor && chooser.input.is_none();
        let style = if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
        lines.push(Line::from(Span::styled(format!(" {option} "), style)));
    }
    if let Some(input) = &chooser.input {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" path: ", theme.fg(theme.palette.text_dim)),
            Span::styled(input.text.clone(), theme.fg(theme.palette.text)),
        ]));
    }
    if let Some(error) = &chooser.error {
        lines.push(Line::from(Span::styled(
            format!(" {error} "),
            theme.fg(theme.palette.danger),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let popup = centered(area, 54, 23);
    if popup.width == 0 || popup.height == 0 {
        return;
    }
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("help", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines = vec![
        Line::from(section("Move", theme)),
        key_line("↑ ↓ / k j", "move in the tree or scroll", theme),
        key_line("← → / h l", "fold the tree or pan sideways", theme),
        key_line("g / G", "jump to the top or bottom", theme),
        Line::from(""),
        Line::from(section("Find", theme)),
        key_line("Ctrl-P", "open the command palette", theme),
        key_line("Ctrl-F", "find a file by name", theme),
        key_line("/ then n / N", "search the view and step matches", theme),
        Line::from(""),
        Line::from(section("View", theme)),
        key_line("Tab", "switch tree and content", theme),
        key_line("m / s", "switch mode or change scope", theme),
        key_line("r / b / t", "edit the revision, base, or target", theme),
        key_line("[ / ] / \\", "resize or reset the tree", theme),
        key_line("? / Esc", "toggle help or dismiss", theme),
        key_line("q / Ctrl-C", "quit", theme),
        Line::from(""),
        Line::from(section("Mouse", theme)),
        key_line("click", "open a file or fold a directory", theme),
        key_line("wheel", "scroll the tree or the content", theme),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_palette(
    frame: &mut Frame,
    area: Rect,
    state: &PaletteState,
    ctx: &RenderCtx,
    theme: &Theme,
) {
    let entries = ctx.entries;
    let width = area.width.saturating_sub(4).min(72);
    let list_height = state.matches.len().min(12);
    let height = (list_height + 3).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 3,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("commands", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines = vec![input_line("› ", &state.input, theme), Line::from("")];
    let offset = window_offset(state.cursor, state.matches.len(), list_height);
    for (row, ranked) in state
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let (label, hint) = entries
            .get(ranked.index)
            .map_or(("", ""), |(_, label, hint)| (*label, *hint));
        lines.push(ranked_line(
            label,
            hint,
            &ranked.positions,
            row == state.cursor,
            width as usize - 2,
            theme,
        ));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no matching command",
            theme.fg(theme.palette.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_finder(
    frame: &mut Frame,
    area: Rect,
    state: &FinderState,
    ctx: &RenderCtx,
    theme: &Theme,
) {
    let width = area.width.saturating_sub(4).min(80);
    let list_height = state.matches.len().min(14);
    let height = (list_height + 3).min(area.height as usize) as u16;
    if width == 0 || height == 0 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 3,
        width,
        height,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);
    let block = popup_block("find file", theme);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let visible = ctx.visible;
    let mut lines = vec![input_line("⌕ ", &state.input, theme), Line::from("")];
    let offset = window_offset(state.cursor, state.matches.len(), list_height);
    for (row, ranked) in state
        .matches
        .iter()
        .enumerate()
        .skip(offset)
        .take(list_height)
    {
        let label = visible
            .get(ranked.index)
            .map_or(String::new(), ToString::to_string);
        lines.push(ranked_line(
            &label,
            "",
            &ranked.positions,
            row == state.cursor,
            width as usize - 2,
            theme,
        ));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no matching file",
            theme.fg(theme.palette.text_muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_search(
    frame: &mut Frame,
    area: Rect,
    state: &SearchState,
    ctx: &RenderCtx,
    theme: &Theme,
) {
    let width = area.width.saturating_sub(4).min(80);
    if width == 0 || area.height < 2 {
        return;
    }
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + area.height - 2,
        width,
        height: 1,
    };
    scrim(frame, area);
    frame.render_widget(Clear, popup);

    let mut spans = input_line("/ ", &state.input, theme).spans;
    let count = ctx.matches;
    let summary = if state.input.value().is_empty() {
        "  type to search".to_owned()
    } else if count == 0 {
        "  no matches".to_owned()
    } else {
        format!("  {count} matches")
    };
    spans.push(Span::styled(summary, theme.fg(theme.palette.text_muted)));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.bg(theme.palette.surface)),
        popup,
    );
}

fn ranked_line(
    label: &str,
    hint: &str,
    positions: &[usize],
    selected: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let base = if selected {
        theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
    } else {
        Style::default()
    };
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_matched = false;
    for (byte, character) in label.char_indices() {
        let matched = positions.binary_search(&byte).is_ok();
        if matched != current_matched && !current.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut current),
                matched_style(current_matched, selected, theme),
            ));
        }
        current_matched = matched;
        current.push(character);
    }
    if !current.is_empty() {
        spans.push(Span::styled(
            current,
            matched_style(current_matched, selected, theme),
        ));
    }

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let hint_width = UnicodeWidthStr::width(hint);
    if used + hint_width + 2 <= width {
        spans.push(Span::styled(" ".repeat(width - used - hint_width), base));
        spans.push(Span::styled(
            hint.to_owned(),
            theme.fg_bg(
                theme.palette.text_muted,
                if selected {
                    theme.palette.selection_bg
                } else {
                    theme.palette.bg
                },
            ),
        ));
    }
    Line::from(spans)
}

fn matched_style(matched: bool, selected: bool, theme: &Theme) -> Style {
    if !matched {
        return if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
    }
    if selected {
        theme.fg_bg(
            theme.ink(theme.palette.match_current_bg),
            theme.palette.match_current_bg,
        )
    } else {
        theme
            .fg(theme.palette.match_fg)
            .add_modifier(Modifier::BOLD)
    }
}

fn section(title: &str, theme: &Theme) -> Span<'static> {
    Span::styled(
        format!(" {title}"),
        theme.fg(theme.palette.accent).add_modifier(Modifier::BOLD),
    )
}

fn key_line(keys: &str, description: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("   {keys:<12}"), theme.fg(theme.palette.text)),
        Span::styled(description.to_owned(), theme.fg(theme.palette.text_dim)),
    ])
}
