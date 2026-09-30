//! The modal overlays that are valid whatever kind of projection is loaded:
//! help, the scope chooser, the mode picker, the command palette, the fuzzy
//! file finder, and search.
//!
//! The revision editors are *not* here: a base-revision prompt only exists on a
//! diff and a show-revision prompt only on a show, so each lives in its content
//! variant.

use base::{AreaSet, ProjectionMode, RepoPath, Selection, SelectionGroup};
use engine::EngineError;

use crate::action::Action;
use crate::content::available_modes;
use crate::fuzzy;
use crate::input::Key;
use crate::text_input::{Edit, TextInput};

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
