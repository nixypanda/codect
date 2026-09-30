// The pure TEA core: `Model`, `Msg`, `Cmd`, and `update`.
//
// Nothing here performs I/O. `update` turns a message and the current model
// into a replacement model plus a list of effects to run. Engine calls only
// ever leave this module as a [`Cmd`], which the runtime interprets. The pure
// `view` lives in [`crate::view`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use base::{
    AreaSet, DiffRowKind, FileDiff, ProjectedFile, ProjectionMode, RepoPath, Selection,
    SelectionGroup,
};
use engine::{CommitStep, EngineError};
use ratatui::layout::{Position, Rect};
use unicode_width::UnicodeWidthStr;

use crate::fuzzy;
use crate::highlight::{self, StyledLine};
use crate::icons::Icons;
use crate::theme::Theme;
use crate::view::diff::layout_diff;
use crate::view::geom::{
    Edge, PaneSlot, body_layout, commit_offset, frame_areas, gutter_width, pane_block,
    split_with_dividers, window_offset,
};

pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

pub(crate) const SINGLE_PANE_MIN_WIDTH: u16 = 40;
pub(crate) const MIN_HEIGHT: u16 = 8;

const TREE_DEFAULT_PERCENT: u16 = 30;
const TREE_MIN_PERCENT: u16 = 15;
const TREE_MAX_PERCENT: u16 = 60;
const TREE_STEP: u16 = 5;

// Which region currently has focus.
//
// `Body` is the single projection pane of a `show`; `Diff` is both diff panes
// treated as one focus unit, so `Tab` only ever toggles between the tree and
// the content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Commits,
    Tree,
    Body,
    Diff,
}

// A key the frontend understands, already translated from a terminal event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Enter,
    Esc,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    CtrlC,
    CtrlD,
    CtrlF,
    CtrlP,
    CtrlU,
}

// A mouse event the frontend understands, already translated from a terminal
// event and normalized to a terminal cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mouse {
    pub column: u16,
    pub row: u16,
    pub kind: MouseKind,
}

// The mouse interactions the frontend acts on. Motion and drag are dropped
// during translation, so they never reach the core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseKind {
    Click,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextInput {
    pub text: String,
    // A byte offset into `text`, always on a character boundary.
    pub cursor: usize,
}

impl TextInput {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn insert(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    pub fn backspace(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().last() {
            self.text.remove(index);
            self.cursor = index;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    pub fn left(&mut self) {
        if let Some((index, _)) = self.text[..self.cursor].char_indices().last() {
            self.cursor = index;
        }
    }

    pub fn right(&mut self) {
        if let Some(character) = self.text[self.cursor..].chars().next() {
            self.cursor += character.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn value(&self) -> String {
        self.text.trim().to_owned()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionField {
    Show,
    Base,
    Target,
}

// A semantic command. Keys and the command palette both produce these, so a
// binding and its palette entry can never drift apart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Quit,
    Help,
    Scope,
    Mode,
    SwitchToRange,
    SwitchToCommits,
    EditShowRevision,
    EditBaseRevision,
    EditTargetRevision,
    NextPane,
    PreviousPane,
    TreeWider,
    TreeNarrower,
    TreeReset,
    Top,
    Bottom,
    PageUp,
    PageDown,
    Palette,
    Finder,
    Search,
    NextMatch,
    PreviousMatch,
}

// A modal interaction that captures keys until it closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    Help,
    Revision {
        field: RevisionField,
        input: TextInput,
    },
    Scope(ScopeChooser),
    Mode {
        cursor: usize,
    },
    Palette(PaletteState),
    Finder(FinderState),
    Search(SearchState),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ranked {
    pub index: usize,
    pub score: i32,
    pub positions: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaletteState {
    pub input: TextInput,
    pub matches: Vec<Ranked>,
    pub cursor: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinderState {
    pub input: TextInput,
    pub matches: Vec<Ranked>,
    pub cursor: usize,
}

// The search input. Live matches are written straight to [`Model::search`] so
// the view previews them while the user types.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchState {
    pub input: TextInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSide {
    Show,
    Old,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub side: SearchSide,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

// A committed search: highlights persist after the input closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Search {
    pub needle: String,
    pub matches: Vec<SearchMatch>,
    pub cursor: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeChooser {
    // `None` while areas are still loading.
    pub areas: Option<AreaSet>,
    pub cursor: usize,
    // `Some` while the user is typing a literal path.
    pub input: Option<TextInput>,
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

    pub(crate) fn options(&self) -> Vec<String> {
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

/// What to project, kept so `m` can re-run the same request in the other mode
/// and so the status bar can label the revisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadRequest {
    Show {
        revision: String,
        mode: ProjectionMode,
        selection: Selection,
    },
    Diff {
        base: String,
        target: String,
        mode: ProjectionMode,
        selection: Selection,
        view: DiffView,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffView {
    Range,
    Commits,
}

impl LoadRequest {
    pub fn mode(&self) -> ProjectionMode {
        match self {
            Self::Show { mode, .. } | Self::Diff { mode, .. } => *mode,
        }
    }

    pub fn with_mode(&self, mode: ProjectionMode) -> Self {
        match self {
            Self::Show {
                revision,
                selection,
                ..
            } => Self::Show {
                revision: revision.clone(),
                mode,
                selection: selection.clone(),
            },
            Self::Diff {
                base,
                target,
                selection,
                view,
                ..
            } => Self::Diff {
                base: base.clone(),
                target: target.clone(),
                mode,
                selection: selection.clone(),
                view: *view,
            },
        }
    }

    /// Replaces the `show` revision.
    pub fn with_revision(&self, revision: String) -> Self {
        match self {
            Self::Show {
                mode, selection, ..
            } => Self::Show {
                revision,
                mode: *mode,
                selection: selection.clone(),
            },
            Self::Diff { .. } => self.clone(),
        }
    }

    /// Replaces the diff base revision.
    pub fn with_base(&self, base: String) -> Self {
        match self {
            Self::Diff {
                target,
                mode,
                selection,
                view,
                ..
            } => Self::Diff {
                base,
                target: target.clone(),
                mode: *mode,
                selection: selection.clone(),
                view: *view,
            },
            Self::Show { .. } => self.clone(),
        }
    }

    /// Replaces the diff target revision.
    pub fn with_target(&self, target: String) -> Self {
        match self {
            Self::Diff {
                base,
                mode,
                selection,
                view,
                ..
            } => Self::Diff {
                base: base.clone(),
                target,
                mode: *mode,
                selection: selection.clone(),
                view: *view,
            },
            Self::Show { .. } => self.clone(),
        }
    }

    /// Replaces the projection scope.
    pub fn with_selection(&self, selection: Selection) -> Self {
        match self {
            Self::Show { revision, mode, .. } => Self::Show {
                revision: revision.clone(),
                mode: *mode,
                selection,
            },
            Self::Diff {
                base,
                target,
                mode,
                view,
                ..
            } => Self::Diff {
                base: base.clone(),
                target: target.clone(),
                mode: *mode,
                selection,
                view: *view,
            },
        }
    }

    /// The selection this request projects.
    pub fn selection(&self) -> &Selection {
        match self {
            Self::Show { selection, .. } | Self::Diff { selection, .. } => selection,
        }
    }

    pub fn with_diff_view(&self, view: DiffView) -> Self {
        let mut request = self.clone();
        if let Self::Diff { view: current, .. } = &mut request {
            *current = view;
        }
        request
    }

    pub fn diff_view(&self) -> Option<DiffView> {
        match self {
            Self::Diff { view, .. } => Some(*view),
            Self::Show { .. } => None,
        }
    }

    /// A short label for the current scope, shown in the status bar.
    pub fn scope_label(&self) -> String {
        let selection = self.selection();
        if selection.groups().is_empty() {
            return "all".to_owned();
        }
        selection
            .groups()
            .iter()
            .map(SelectionGroup::label)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// The loaded projection, either a single revision or a two-revision diff.
//
// The payloads sit behind an `Arc` so cloning a `Model` is O(1) regardless of
// how many files or bytes a projection holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Content {
    Show(Arc<[ProjectedFile]>),
    Diff(Arc<[FileDiff]>),
}

impl From<Vec<ProjectedFile>> for Content {
    fn from(files: Vec<ProjectedFile>) -> Self {
        Self::Show(files.into())
    }
}

impl From<Vec<FileDiff>> for Content {
    fn from(diffs: Vec<FileDiff>) -> Self {
        Self::Diff(diffs.into())
    }
}

impl Content {
    // Paths the tree shows. A show hides files whose projection is empty; a
    // diff already contains only paths whose projections differ.
    fn visible_paths(&self) -> Vec<RepoPath> {
        match self {
            Self::Show(files) => files
                .iter()
                .filter(|file| !file.canonical_text().is_empty())
                .map(|file| file.path().clone())
                .collect(),
            Self::Diff(diffs) => diffs.iter().map(|diff| diff.path().clone()).collect(),
        }
    }
}

#[derive(Debug)]
pub enum Msg {
    Key(Key),
    Mouse(Mouse),
    Resize {
        width: u16,
        height: u16,
    },
    // A periodic tick, used to animate the spinner and expire a diagnostic.
    Tick,
    // A projection effect finished. The request it ran for is echoed back.
    Loaded {
        request: LoadRequest,
        result: Result<Content, Box<EngineError>>,
    },
    HistoryLoaded {
        request: LoadRequest,
        result: Result<(Vec<CommitStep>, Content), Box<EngineError>>,
    },
    StepLoaded {
        request: LoadRequest,
        index: usize,
        result: Result<Content, Box<EngineError>>,
    },
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

// An effect the runtime must interpret. I/O is data, never a side effect of
// `update`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    Load {
        request: LoadRequest,
    },
    LoadStep {
        request: LoadRequest,
        index: usize,
        step: CommitStep,
    },
    LoadAreas,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowKind {
    Directory { path: RepoPath, expanded: bool },
    File { path: RepoPath },
}

impl RowKind {
    fn path(&self) -> &RepoPath {
        match self {
            RowKind::Directory { path, .. } | RowKind::File { path } => path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    pub kind: RowKind,
}

// The kind of a rendered diff row: either an aligned diff row or a synthetic
// hunk header inserted where context was collapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowKind {
    Diff(DiffRowKind),
    Hunk,
    // A gap between hunks: a count of unchanged rows that were collapsed.
    Collapse(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

// One visual row of a wrapped side-by-side diff.
//
// A logical aligned row with a wrapped side expands into several `VisualRow`s;
// only the first carries line numbers and the rest are continuation rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub kind: VisualRowKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_runs: StyledLine,
    pub new_runs: StyledLine,
    pub continuation: bool,
}

#[derive(Clone, Default)]
pub struct DiffHighlight {
    pub old: Vec<StyledLine>,
    pub new: Vec<StyledLine>,
}

// Highlighted lines for the selected file, keyed by path.
//
// Highlighting is pure but not free, so it is computed once per file when the
// selection first reaches it and reused while the user stays on that file.
//
// Both maps are bounded: browsing a large tree touches many files, and an
// unbounded cache would grow without limit and make every `Model` clone
// proportional to how far the user has scrolled.
#[derive(Clone, Default)]
struct HighlightCache {
    show: BoundedCache<Arc<Vec<StyledLine>>>,
    diff: BoundedCache<Arc<DiffHighlight>>,
}

const HIGHLIGHT_CACHE_CAP: usize = 32;

// A small insertion-ordered map that evicts its oldest entry past a cap.
//
// Eviction is by insertion order rather than true recency: entries are read
// without `&mut self` from `view`, and for navigation the insertion order is
// the access order.
#[derive(Clone)]
struct BoundedCache<V> {
    entries: BTreeMap<RepoPath, V>,
    order: VecDeque<RepoPath>,
}

impl<V> Default for BoundedCache<V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<V> BoundedCache<V> {
    fn get(&self, path: &RepoPath) -> Option<&V> {
        self.entries.get(path)
    }

    fn contains_key(&self, path: &RepoPath) -> bool {
        self.entries.contains_key(path)
    }

    fn insert(&mut self, path: RepoPath, value: V) {
        if self.entries.insert(path.clone(), value).is_none() {
            self.order.push_back(path);
        }
        while self.order.len() > HIGHLIGHT_CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct DerivedKey {
    generation: u64,
    width: u16,
    height: u16,
    selected: Option<RepoPath>,
    tree_percent: u16,
}

// Expensive rendering state derived from the model, cached so a frame or a
// scroll key does not recompute it. Held behind an `Arc`, so cloning a `Model`
// never clones the layout.
#[derive(Clone, Default)]
struct Derived {
    key: Option<DerivedKey>,
    diff_rows: Vec<VisualRow>,
}

// The entire UI state. It is replaced wholesale by `update`, never edited in
// place by anything else.
//
// Every large collection sits behind an `Arc`, so a clone is O(1); the only
// deep data is small (fold state, highlight-cache entries).
#[derive(Clone)]
pub struct Model {
    pub root: String,
    // The request that produced `content`; retained so `m` can re-project.
    pub request: LoadRequest,
    pub mode: ProjectionMode,
    pub scope_label: String,
    pub content: Content,
    pub commits: Arc<[CommitStep]>,
    // Navigation target while input is being batched; the displayed diff still
    // belongs to `commit_cursor` until its projection succeeds.
    pub commit_target: Option<usize>,
    pub commit_cursor: usize,
    pub commit_scroll: usize,
    // Paths the tree shows, in raw path-byte order.
    pub visible: Arc<[RepoPath]>,
    // Directories the user folded. Everything is expanded by default.
    pub collapsed: BTreeSet<RepoPath>,
    pub rows: Arc<[TreeRow]>,
    pub cursor: usize,
    pub selected: Option<RepoPath>,
    pub focus: Pane,
    pub body_scroll: u16,
    pub body_hscroll: u16,
    pub tree_percent: u16,
    // The active modal overlay, if any. It captures keys until it closes.
    pub overlay: Option<Overlay>,
    // A committed in-pane search whose highlights persist.
    pub search: Option<Search>,
    pub diagnostic: Option<String>,
    highlights: HighlightCache,
    // Bumped whenever `content` is replaced, invalidating [`Derived`].
    generation: u64,
    derived: Arc<Derived>,
    pub width: u16,
    pub height: u16,
    // The resolved design tokens; view code never names a raw color.
    pub theme: Theme,
    // The resolved tree glyph set; icons are opt-in.
    pub icons: Icons,
    // A load in flight, so the view can show a spinner.
    pub pending: Option<LoadRequest>,
    // The spinner animation frame, advanced by [`Msg::Tick`].
    pub spinner: u8,
    pub diagnostic_ttl: u8,
    pub quit: bool,
}

impl Model {
    fn navigation_commit(&self) -> usize {
        self.commit_target.unwrap_or(self.commit_cursor)
    }

    pub fn diff_revisions(&self) -> Option<(String, String)> {
        let LoadRequest::Diff {
            base, target, view, ..
        } = &self.request
        else {
            return None;
        };
        if *view == DiffView::Commits
            && let Some(step) = self.commits.get(self.commit_cursor)
        {
            let parent = step.parent_id.to_string();
            let commit = step.commit_id.to_string();
            return Some((
                parent[..parent.len().min(7)].to_owned(),
                commit[..commit.len().min(7)].to_owned(),
            ));
        }
        Some((base.clone(), target.clone()))
    }

    pub fn new(
        root: String,
        request: LoadRequest,
        scope_label: String,
        width: u16,
        height: u16,
        theme: Theme,
        icons: Icons,
    ) -> Self {
        let mode = request.mode();
        Self {
            root,
            pending: Some(request.clone()),
            request,
            mode,
            scope_label,
            content: Content::Show(Arc::from(Vec::new())),
            commits: Arc::from(Vec::new()),
            commit_target: None,
            commit_cursor: 0,
            commit_scroll: 0,
            visible: Arc::from(Vec::new()),
            collapsed: BTreeSet::new(),
            rows: Arc::from(Vec::new()),
            cursor: 0,
            selected: None,
            focus: Pane::Tree,
            body_scroll: 0,
            body_hscroll: 0,
            tree_percent: TREE_DEFAULT_PERCENT,
            overlay: None,
            search: None,
            diagnostic: None,
            highlights: HighlightCache::default(),
            generation: 0,
            derived: Arc::new(Derived::default()),
            width,
            height,
            theme,
            icons,
            spinner: 0,
            diagnostic_ttl: 0,
            quit: false,
        }
    }

    pub fn active_file(&self) -> Option<&ProjectedFile> {
        let path = self.selected.as_ref()?;
        match &self.content {
            Content::Show(files) => files.iter().find(|file| file.path() == path),
            Content::Diff(_) => None,
        }
    }

    pub fn active_text(&self) -> Option<&str> {
        self.active_file().map(ProjectedFile::canonical_text)
    }

    pub fn active_diff(&self) -> Option<&FileDiff> {
        let path = self.selected.as_ref()?;
        match &self.content {
            Content::Diff(diffs) => diffs.iter().find(|diff| diff.path() == path),
            Content::Show(_) => None,
        }
    }

    pub(crate) fn active_show_lines(&self) -> Option<&[StyledLine]> {
        let path = self.selected.as_ref()?;
        self.highlights.show.get(path).map(|lines| lines.as_slice())
    }

    pub(crate) fn active_diff_highlight(&self) -> Option<&DiffHighlight> {
        let path = self.selected.as_ref()?;
        self.highlights.diff.get(path).map(|lines| lines.as_ref())
    }

    pub(crate) fn diff_rows(&self) -> &[VisualRow] {
        self.derived.diff_rows.as_slice()
    }

    pub(crate) fn change_kind(&self, path: &RepoPath) -> Option<ChangeKind> {
        let Content::Diff(diffs) = &self.content else {
            return None;
        };
        let diff = diffs.iter().find(|diff| diff.path() == path)?;
        Some(match diff {
            FileDiff::Added { .. } => ChangeKind::Added,
            FileDiff::Deleted { .. } => ChangeKind::Deleted,
            FileDiff::Modified { .. } => ChangeKind::Modified,
        })
    }

    pub(crate) fn search_ranges(&self, side: SearchSide, line: usize) -> Vec<(usize, usize, bool)> {
        let Some(search) = &self.search else {
            return Vec::new();
        };
        search
            .matches
            .iter()
            .enumerate()
            .filter(|(_, matched)| matched.side == side && matched.line == line)
            .map(|(index, matched)| (matched.start, matched.end, index == search.cursor))
            .collect()
    }

    // Recomputes a committed search for the current selection, so switching
    // files never leaves highlights pointing at the previous file's lines.
    fn resync_search(&mut self) {
        let Some(needle) = self.search.as_ref().map(|search| search.needle.clone()) else {
            return;
        };
        let matches = search_matches(self, &needle);
        if let Some(search) = &mut self.search {
            search.matches = matches;
            search.cursor = search.cursor.min(search.matches.len().saturating_sub(1));
        }
    }

    fn ensure_highlight(&mut self) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        match &self.content {
            Content::Show(files) => {
                if self.highlights.show.contains_key(&path) {
                    return;
                }
                let Some(file) = files.iter().find(|file| file.path() == &path) else {
                    return;
                };
                let lines =
                    highlight::highlight(file.canonical_text(), file.language(), &self.theme);
                self.highlights.show.insert(path, Arc::new(lines));
            }
            Content::Diff(diffs) => {
                if self.highlights.diff.contains_key(&path) {
                    return;
                }
                let Some(diff) = diffs.iter().find(|diff| *diff.path() == path) else {
                    return;
                };
                let language = match diff {
                    FileDiff::Added { new } => new.language(),
                    FileDiff::Deleted { old } => old.language(),
                    FileDiff::Modified { old, .. } => old.language(),
                };
                let highlight_side = |file: &ProjectedFile| {
                    highlight::highlight(file.canonical_text(), language, &self.theme)
                };
                let (old, new) = match diff {
                    FileDiff::Added { new } => (Vec::new(), highlight_side(new)),
                    FileDiff::Deleted { old } => (highlight_side(old), Vec::new()),
                    FileDiff::Modified { old, new } => (highlight_side(old), highlight_side(new)),
                };
                self.highlights
                    .diff
                    .insert(path, Arc::new(DiffHighlight { old, new }));
            }
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.active_text().map_or(0, |text| text.lines().count())
    }

    fn max_line_width(&self) -> usize {
        self.active_text().map_or(0, |text| {
            text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0)
        })
    }

    fn current_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.cursor)
    }

    fn refresh_derived(&mut self) {
        let key = DerivedKey {
            generation: self.generation,
            width: self.width,
            height: self.height,
            selected: self.selected.clone(),
            tree_percent: self.tree_percent,
        };
        if self.derived.key.as_ref() == Some(&key) {
            return;
        }
        let diff_rows = self.compute_diff_rows();
        self.derived = Arc::new(Derived {
            key: Some(key),
            diff_rows,
        });
    }

    pub(crate) fn compute_diff_rows(&self) -> Vec<VisualRow> {
        let Some(diff) = self.active_diff() else {
            return Vec::new();
        };
        let (_, content, _) = frame_areas(self.width, self.height);
        let (old_width, new_width) = self.diff_side_widths(content, diff);
        let empty = DiffHighlight::default();
        let highlights = self.active_diff_highlight().unwrap_or(&empty);
        layout_diff(
            diff,
            old_width,
            new_width,
            &highlights.old,
            &highlights.new,
            &self.theme,
        )
    }

    fn diff_side_widths(&self, content: Rect, diff: &FileDiff) -> (usize, usize) {
        let gutter = gutter_width(diff);
        if self.width >= SIDE_BY_SIDE_MIN_WIDTH {
            let rest = 100 - self.tree_percent;
            let side = rest / 2;
            let (columns, _) =
                split_with_dividers(content, &[self.tree_percent, side, rest - side]);
            let old_inner = pane_block("", false, &self.theme, Edge::Middle).inner(columns[1]);
            let new_inner = pane_block("", false, &self.theme, Edge::Right).inner(columns[2]);
            (
                (old_inner.width as usize).saturating_sub(gutter),
                (new_inner.width as usize).saturating_sub(gutter),
            )
        } else {
            let inner = pane_block("", false, &self.theme, Edge::Solo).inner(content);
            let width = (inner.width as usize).saturating_sub(gutter);
            (width, width)
        }
    }

    // Installs a freshly loaded projection, preserving the selected path when
    // it is still visible and choosing the nearest visible file otherwise.
    fn install(&mut self, content: Content, keep_selection: bool) {
        let previous = self.selected.clone();
        let hint = self.cursor;

        self.pending = None;
        self.visible = Arc::from(content.visible_paths());
        self.content = content;
        self.generation = self.generation.wrapping_add(1);
        self.highlights = HighlightCache::default();
        self.rows = Arc::from(build_rows(&self.visible, &self.collapsed));
        // Drop fold state for directories that no longer exist.
        self.collapsed
            .retain(|path| self.rows.iter().any(|row| row.kind.path() == path));

        let target = if keep_selection {
            previous.filter(|path| self.visible.contains(path))
        } else {
            None
        }
        .or_else(|| self.nearest_visible(hint));

        match target {
            Some(path) => {
                self.selected = Some(path.clone());
                self.cursor = self.row_of_file(&path).unwrap_or(0);
            }
            None => {
                self.selected = None;
                self.cursor = self.first_file_row().unwrap_or(0);
            }
        }

        if !focus_order(self).contains(&self.focus) {
            self.focus = Pane::Tree;
        }
        self.body_scroll = 0;
        self.body_hscroll = 0;
        // Highlighting, search re-sync, and diff layout are deferred to
        // [`settle`], which the runtime runs once per input batch.
    }

    fn nearest_visible(&self, hint: usize) -> Option<RepoPath> {
        let file_rows: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row.kind, RowKind::File { .. }))
            .map(|(index, _)| index)
            .collect();
        let chosen = file_rows
            .iter()
            .copied()
            .find(|&index| index >= hint)
            .or_else(|| file_rows.last().copied())?;
        Some(self.rows[chosen].kind.path().clone())
    }

    fn row_of_file(&self, path: &RepoPath) -> Option<usize> {
        self.rows.iter().position(
            |row| matches!(&row.kind, RowKind::File { path: candidate } if candidate == path),
        )
    }

    fn first_file_row(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row.kind, RowKind::File { .. }))
    }

    fn row_of_directory(&self, path: &RepoPath) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(&row.kind, RowKind::Directory { path: candidate, .. } if candidate == path)
        })
    }
}

fn focus_order(model: &Model) -> &'static [Pane] {
    if model.request.diff_view() == Some(DiffView::Commits) {
        return &[Pane::Commits, Pane::Tree, Pane::Diff];
    }
    match model.content {
        Content::Show(_) => &[Pane::Tree, Pane::Body],
        Content::Diff(_) => &[Pane::Tree, Pane::Diff],
    }
}

const DIAGNOSTIC_TICKS: u8 = 40;

// The pure update function. It performs no I/O and reads no external state.
pub fn update(msg: Msg, model: &Model) -> (Model, Vec<Cmd>) {
    let mut next = model.clone();
    let mut cmds = Vec::new();

    match msg {
        Msg::Key(key) => handle_key(key, &mut next, &mut cmds),
        Msg::Mouse(event) => handle_mouse(event, &mut next, &mut cmds),
        Msg::Resize { width, height } => {
            next.width = width;
            next.height = height;
            clamp_view(&mut next);
        }
        Msg::Tick => {
            next.spinner = next.spinner.wrapping_add(1);
            if next.diagnostic.is_some() {
                next.diagnostic_ttl = next.diagnostic_ttl.saturating_sub(1);
                if next.diagnostic_ttl == 0 {
                    next.diagnostic = None;
                }
            }
        }
        Msg::Loaded { request, result } => {
            next.pending = None;
            match result {
                Ok(content) => {
                    let keep_commits = matches!((&next.request, &request),
                        (LoadRequest::Diff { base: old_base, target: old_target, .. },
                         LoadRequest::Diff { base: new_base, target: new_target, view: DiffView::Range, .. })
                         if old_base == new_base && old_target == new_target);
                    next.mode = request.mode();
                    next.scope_label = request.scope_label();
                    next.request = request;
                    if !keep_commits {
                        next.commits = Arc::from(Vec::new());
                        next.commit_target = None;
                        next.commit_cursor = 0;
                        next.commit_scroll = 0;
                    }
                    next.install(content, true);
                    next.diagnostic = None;
                    next.diagnostic_ttl = 0;
                }
                // A failed replacement leaves the previous model untouched.
                Err(error) => {
                    next.diagnostic = Some(error.to_string());
                    next.diagnostic_ttl = DIAGNOSTIC_TICKS;
                }
            }
        }
        Msg::HistoryLoaded { request, result } => {
            next.pending = None;
            match result {
                Ok((steps, content)) => {
                    let preferred = match (&next.request, &request) {
                        (
                            LoadRequest::Diff {
                                base: old_base,
                                target: old_target,
                                ..
                            },
                            LoadRequest::Diff {
                                base: new_base,
                                target: new_target,
                                ..
                            },
                        ) if old_base == new_base && old_target == new_target => next
                            .commits
                            .get(next.commit_cursor)
                            .map(|step| step.commit_id.clone()),
                        _ => None,
                    };
                    next.mode = request.mode();
                    next.scope_label = request.scope_label();
                    next.request = request;
                    next.commits = steps.into();
                    next.commit_target = None;
                    next.commit_cursor = 0;
                    next.commit_scroll = 0;
                    next.install(content, true);
                    next.diagnostic = None;
                    next.diagnostic_ttl = 0;
                    next.focus = Pane::Commits;
                    if let Some(index) = preferred
                        .and_then(|id| next.commits.iter().position(|step| step.commit_id == id))
                        && index > 0
                    {
                        cmds.push(Cmd::LoadStep {
                            request: next.request.clone(),
                            index,
                            step: next.commits[index].clone(),
                        });
                    }
                }
                Err(error) => {
                    next.diagnostic = Some(error.to_string());
                    next.diagnostic_ttl = DIAGNOSTIC_TICKS;
                }
            }
        }
        Msg::StepLoaded {
            request,
            index,
            result,
        } => {
            next.pending = None;
            next.commit_target = None;
            match result {
                Ok(content) if request == next.request && index < next.commits.len() => {
                    next.commit_cursor = index;
                    next.install(content, true);
                    sync_commit_scroll(&mut next);
                    next.diagnostic = None;
                    next.diagnostic_ttl = 0;
                }
                Err(error) => {
                    next.diagnostic = Some(error.to_string());
                    next.diagnostic_ttl = DIAGNOSTIC_TICKS;
                }
                _ => {}
            }
        }
        Msg::AreasLoaded(result) => {
            if let Some(Overlay::Scope(chooser)) = &mut next.overlay {
                match result {
                    Ok(areas) => {
                        chooser.cursor = chooser.cursor.min(areas.names().count());
                        chooser.areas = Some(areas);
                        chooser.error = None;
                    }
                    Err(error) => chooser.error = Some(error.to_string()),
                }
            }
        }
    }

    // A newly emitted load marks the model busy so the view can spin.
    if let Some(request) = cmds.iter().find_map(|cmd| match cmd {
        Cmd::Load { request } | Cmd::LoadStep { request, .. } => Some(request),
        Cmd::LoadAreas => None,
    }) {
        next.pending = Some(request.clone());
    }

    (next, cmds)
}

// Replaces all step projections in one input batch with its final target.
// A full reload wins when mode, scope, endpoints, or view also changed.
pub(crate) fn coalesce_commit_loads(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let had_step = cmds.iter().any(|cmd| matches!(cmd, Cmd::LoadStep { .. }));
    if !had_step {
        return;
    }
    cmds.retain(|cmd| !matches!(cmd, Cmd::LoadStep { .. }));
    if let Some(request) = cmds.iter().rev().find_map(|cmd| match cmd {
        Cmd::Load { request } => Some(request),
        _ => None,
    }) {
        model.commit_target = None;
        model.pending = Some(request.clone());
        return;
    }
    if let Some(index) = model.commit_target
        && let Some(step) = model.commits.get(index)
    {
        cmds.push(Cmd::LoadStep {
            request: model.request.clone(),
            index,
            step: step.clone(),
        });
    } else {
        model.pending = None;
    }
}

// Runs the selection-dependent work a frame needs, once per input batch.
//
// Highlighting, search re-sync, and the wrapped diff layout are pure but not
// free, and they depend only on the *final* selection of a batch. Running them
// here rather than inside `update` means a burst of tree navigation pays for
// one file instead of every row the cursor passed over.
//
// The runtime calls this after folding a batch of messages and before drawing.
pub(crate) fn settle(mut model: Model) -> Model {
    if body_is_visible(&model) {
        model.ensure_highlight();
    }
    model.resync_search();
    model.refresh_derived();
    model
}

// Whether the current layout draws the content pane at all.
//
// In a narrow terminal with the tree focused, the body is not drawn, so
// highlighting the selection would be wasted work.
fn body_is_visible(model: &Model) -> bool {
    if model.width < SINGLE_PANE_MIN_WIDTH || model.height < MIN_HEIGHT {
        return false;
    }
    model.width >= SIDE_BY_SIDE_MIN_WIDTH || model.focus != Pane::Tree
}

fn handle_key(key: Key, model: &mut Model, cmds: &mut Vec<Cmd>) {
    // Ctrl-C always quits; `q` quits except while an input overlay is capturing.
    if key == Key::CtrlC {
        model.quit = true;
        return;
    }
    let quit_with_q = matches!(model.overlay, None | Some(Overlay::Help));
    if key == Key::Char('q') && quit_with_q {
        model.quit = true;
        return;
    }
    if model.overlay.is_some() {
        overlay_key(key, model, cmds);
        return;
    }
    if let Some(action) = command_for(key, model) {
        apply_action(action, model, cmds);
        return;
    }
    match key {
        Key::Esc => {
            model.diagnostic = None;
            model.diagnostic_ttl = 0;
            model.search = None;
        }
        _ => match model.focus {
            Pane::Commits => commits_key(key, model, cmds),
            Pane::Tree => tree_key(key, model),
            Pane::Body | Pane::Diff => body_key(key, model),
        },
    }
}

// The semantic command a key produces outside an overlay, if any. Navigation
// keys are handled separately so they stay context sensitive.
fn command_for(key: Key, model: &Model) -> Option<Action> {
    let show = matches!(model.content, Content::Show(_));
    let diff = matches!(model.content, Content::Diff(_));
    match key {
        Key::Char('?') => Some(Action::Help),
        Key::Char('s') => Some(Action::Scope),
        Key::Char('m') => Some(Action::Mode),
        Key::Char('r') if show => Some(Action::EditShowRevision),
        Key::Char('b') if diff => Some(Action::EditBaseRevision),
        Key::Char('t') if diff => Some(Action::EditTargetRevision),
        Key::Char('[') => Some(Action::TreeNarrower),
        Key::Char(']') => Some(Action::TreeWider),
        Key::Char('\\') => Some(Action::TreeReset),
        Key::Tab => Some(Action::NextPane),
        Key::BackTab => Some(Action::PreviousPane),
        Key::Char('g') => Some(Action::Top),
        Key::Char('G') => Some(Action::Bottom),
        Key::Char('n') => Some(Action::NextMatch),
        Key::Char('N') => Some(Action::PreviousMatch),
        Key::CtrlP => Some(Action::Palette),
        Key::CtrlF => Some(Action::Finder),
        Key::Char('/') => Some(Action::Search),
        Key::PageDown | Key::CtrlD => Some(Action::PageDown),
        Key::PageUp | Key::CtrlU => Some(Action::PageUp),
        _ => None,
    }
}

// Performs a semantic command. Shared by keys and the command palette.
fn apply_action(action: Action, model: &mut Model, cmds: &mut Vec<Cmd>) {
    match action {
        Action::Quit => model.quit = true,
        Action::Help => model.overlay = Some(Overlay::Help),
        Action::Scope => {
            model.overlay = Some(Overlay::Scope(ScopeChooser::loading()));
            cmds.push(Cmd::LoadAreas);
        }
        Action::Mode => {
            let modes = available_modes();
            let cursor = modes
                .iter()
                .position(|mode| *mode == model.mode)
                .unwrap_or(0);
            model.overlay = Some(Overlay::Mode { cursor });
        }
        Action::SwitchToRange | Action::SwitchToCommits => {
            let view = if action == Action::SwitchToRange {
                DiffView::Range
            } else {
                DiffView::Commits
            };
            if model
                .request
                .diff_view()
                .is_some_and(|current| current != view)
            {
                cmds.push(Cmd::Load {
                    request: model.request.with_diff_view(view),
                });
            }
        }
        Action::EditShowRevision if matches!(model.content, Content::Show(_)) => {
            model.overlay = Some(Overlay::Revision {
                field: RevisionField::Show,
                input: TextInput::new(current_revision(model)),
            });
        }
        Action::EditBaseRevision if matches!(model.content, Content::Diff(_)) => {
            model.overlay = Some(Overlay::Revision {
                field: RevisionField::Base,
                input: TextInput::new(current_base(model)),
            });
        }
        Action::EditTargetRevision if matches!(model.content, Content::Diff(_)) => {
            model.overlay = Some(Overlay::Revision {
                field: RevisionField::Target,
                input: TextInput::new(current_target(model)),
            });
        }
        Action::NextPane => cycle_focus(model, true),
        Action::PreviousPane => cycle_focus(model, false),
        Action::TreeWider => {
            model.tree_percent = (model.tree_percent + TREE_STEP).min(TREE_MAX_PERCENT);
        }
        Action::TreeNarrower => {
            model.tree_percent = model
                .tree_percent
                .saturating_sub(TREE_STEP)
                .max(TREE_MIN_PERCENT);
        }
        Action::TreeReset => model.tree_percent = TREE_DEFAULT_PERCENT,
        Action::Top => {
            if model.focus == Pane::Commits {
                select_commit(model, 0, cmds);
            } else if model.focus == Pane::Tree {
                move_cursor_to(model, 0);
            } else {
                model.body_scroll = 0;
            }
        }
        Action::Bottom => {
            if model.focus == Pane::Commits {
                select_commit(model, model.commits.len().saturating_sub(1), cmds);
            } else if model.focus == Pane::Tree {
                let last = model.rows.len().saturating_sub(1);
                move_cursor_to(model, last);
            } else {
                model.body_scroll = max_scroll(model);
            }
        }
        Action::PageUp => {
            let step = page_step(model);
            if model.focus == Pane::Commits {
                select_commit(
                    model,
                    model.navigation_commit().saturating_sub(step as usize),
                    cmds,
                );
            } else if model.focus == Pane::Tree {
                move_cursor(model, -i32::from(step));
            } else {
                model.body_scroll = model.body_scroll.saturating_sub(step);
            }
        }
        Action::PageDown => {
            let step = page_step(model);
            if model.focus == Pane::Commits {
                select_commit(
                    model,
                    model.navigation_commit().saturating_add(step as usize),
                    cmds,
                );
            } else if model.focus == Pane::Tree {
                move_cursor(model, i32::from(step));
            } else {
                model.body_scroll = model
                    .body_scroll
                    .saturating_add(step)
                    .min(max_scroll(model));
            }
        }
        Action::Palette => model.overlay = Some(Overlay::Palette(PaletteState::new(model))),
        Action::Finder => model.overlay = Some(Overlay::Finder(FinderState::new(model))),
        Action::Search => model.overlay = Some(Overlay::Search(SearchState::new())),
        Action::NextMatch => step_search(model, true),
        Action::PreviousMatch => step_search(model, false),
        // Revision actions are ignored when the content does not support them.
        Action::EditShowRevision | Action::EditBaseRevision | Action::EditTargetRevision => {}
    }
}

fn page_step(model: &Model) -> u16 {
    (model.height.saturating_sub(3) / 2).max(1)
}

fn move_cursor_to(model: &mut Model, index: usize) {
    if model.rows.is_empty() {
        return;
    }
    model.cursor = index.min(model.rows.len() - 1);
    sync_selected(model);
}

// ---------------------------------------------------------------------------
// Palette, finder, and search
// ---------------------------------------------------------------------------

pub(crate) fn palette_entries(model: &Model) -> Vec<(Action, &'static str, &'static str)> {
    let mut entries = vec![
        (Action::Mode, "Switch Types / Signatures", "m"),
        (Action::Scope, "Change scope", "s"),
    ];
    match &model.content {
        Content::Show(_) => entries.push((Action::EditShowRevision, "Edit revision", "r")),
        Content::Diff(_) => {
            entries.push((Action::EditBaseRevision, "Edit base revision", "b"));
            entries.push((Action::EditTargetRevision, "Edit target revision", "t"));
            match model.request.diff_view() {
                Some(DiffView::Range) => {
                    entries.push((Action::SwitchToCommits, "Switch to commits view", ""))
                }
                Some(DiffView::Commits) => {
                    entries.push((Action::SwitchToRange, "Switch to range view", ""))
                }
                None => {}
            }
        }
    }
    entries.extend([
        (Action::Finder, "Find file", "Ctrl-F"),
        (Action::Search, "Search in view", "/"),
        (Action::NextPane, "Switch pane", "Tab"),
        (Action::TreeWider, "Widen file tree", "]"),
        (Action::TreeNarrower, "Narrow file tree", "["),
        (Action::TreeReset, "Reset file tree", "\\"),
        (Action::Top, "Jump to top", "g"),
        (Action::Bottom, "Jump to bottom", "G"),
        (Action::Help, "Help", "?"),
        (Action::Quit, "Quit", "q"),
    ]);
    entries
}

impl PaletteState {
    fn new(model: &Model) -> Self {
        let mut state = Self {
            input: TextInput::new(""),
            matches: Vec::new(),
            cursor: 0,
        };
        state.refresh(model);
        state
    }

    fn refresh(&mut self, model: &Model) {
        let needle = self.input.text.clone();
        let entries = palette_entries(model);
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
    fn new(model: &Model) -> Self {
        let mut state = Self {
            input: TextInput::new(""),
            matches: Vec::new(),
            cursor: 0,
        };
        state.refresh(model);
        state
    }

    fn refresh(&mut self, model: &Model) {
        let needle = self.input.text.clone();
        let mut matches: Vec<Ranked> = model
            .visible
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

impl SearchState {
    fn new() -> Self {
        Self {
            input: TextInput::new(""),
        }
    }
}

fn refresh_search(model: &mut Model, needle: &str) {
    if needle.is_empty() {
        model.search = None;
        return;
    }
    let matches = search_matches(model, needle);
    model.search = Some(Search {
        needle: needle.to_owned(),
        matches,
        cursor: 0,
    });
}

fn search_matches(model: &Model, needle: &str) -> Vec<SearchMatch> {
    let needle_lower = needle.to_lowercase();
    if needle_lower.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    match &model.content {
        Content::Show(_) => {
            if let Some(file) = model.active_file() {
                collect_matches(
                    file.canonical_text(),
                    &needle_lower,
                    SearchSide::Show,
                    &mut matches,
                );
            }
        }
        Content::Diff(_) => {
            if let Some(diff) = model.active_diff() {
                match diff {
                    FileDiff::Added { new } => collect_matches(
                        new.canonical_text(),
                        &needle_lower,
                        SearchSide::New,
                        &mut matches,
                    ),
                    FileDiff::Deleted { old } => collect_matches(
                        old.canonical_text(),
                        &needle_lower,
                        SearchSide::Old,
                        &mut matches,
                    ),
                    FileDiff::Modified { old, new } => {
                        collect_matches(
                            old.canonical_text(),
                            &needle_lower,
                            SearchSide::Old,
                            &mut matches,
                        );
                        collect_matches(
                            new.canonical_text(),
                            &needle_lower,
                            SearchSide::New,
                            &mut matches,
                        );
                    }
                }
            }
        }
    }
    matches
}

fn collect_matches(text: &str, needle_lower: &str, side: SearchSide, out: &mut Vec<SearchMatch>) {
    let needle: Vec<char> = needle_lower.chars().collect();
    if needle.is_empty() {
        return;
    }
    for (index, line) in text.lines().enumerate() {
        let hay: Vec<(usize, char)> = line.char_indices().collect();
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            let matched = (0..needle.len())
                .all(|k| hay[i + k].1.to_lowercase().next().unwrap_or(hay[i + k].1) == needle[k]);
            if matched {
                let start = hay[i].0;
                let end = hay
                    .get(i + needle.len())
                    .map_or(line.len(), |&(byte, _)| byte);
                out.push(SearchMatch {
                    side,
                    line: index + 1,
                    start,
                    end,
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
}

fn focus_current_match(model: &mut Model) {
    if let Some(search) = &model.search
        && let Some(matched) = search.matches.get(search.cursor)
    {
        let matched = matched.clone();
        scroll_to_match(model, &matched);
    }
}

fn step_search(model: &mut Model, forward: bool) {
    let matched = match &model.search {
        Some(search) if !search.matches.is_empty() => {
            let len = search.matches.len();
            let cursor = if forward {
                (search.cursor + 1) % len
            } else {
                (search.cursor + len - 1) % len
            };
            (cursor, search.matches[cursor].clone())
        }
        _ => return,
    };
    if let Some(search) = &mut model.search {
        search.cursor = matched.0;
    }
    scroll_to_match(model, &matched.1);
}

fn scroll_to_match(model: &mut Model, matched: &SearchMatch) {
    let height = model.height.saturating_sub(4) as usize;
    let index = match model.content {
        Content::Show(_) => matched.line.saturating_sub(1),
        Content::Diff(_) => {
            let side = matched.side;
            model
                .diff_rows()
                .iter()
                .position(|row| match side {
                    SearchSide::Old => row.old_number == Some(matched.line),
                    SearchSide::New => row.new_number == Some(matched.line),
                    SearchSide::Show => false,
                })
                .unwrap_or(0)
        }
    };
    let scroll = model.body_scroll as usize;
    if index < scroll {
        model.body_scroll = index as u16;
    } else if height > 0 && index >= scroll + height {
        model.body_scroll = (index + 1 - height) as u16;
    }
}

// Routes a key to the active overlay. The overlay is taken and reinserted so
// its owned editor state can be mutated.
fn overlay_key(key: Key, model: &mut Model, cmds: &mut Vec<Cmd>) {
    let Some(overlay) = model.overlay.take() else {
        return;
    };
    match overlay {
        Overlay::Help => {
            // A dismissal key closes help; every other key is swallowed.
            if key != Key::Esc && key != Key::Char('?') {
                model.overlay = Some(Overlay::Help);
            }
        }
        Overlay::Revision { field, mut input } => {
            let mut reopen = true;
            match key {
                Key::Esc => reopen = false,
                Key::Enter => {
                    reopen = false;
                    let value = input.value();
                    if !value.is_empty() {
                        let request = match field {
                            RevisionField::Show => model.request.with_revision(value),
                            RevisionField::Base => model.request.with_base(value),
                            RevisionField::Target => model.request.with_target(value),
                        };
                        cmds.push(Cmd::Load { request });
                    }
                }
                Key::Char(character) => input.insert(character),
                Key::Backspace => input.backspace(),
                Key::Delete => input.delete(),
                Key::Left => input.left(),
                Key::Right => input.right(),
                Key::Home => input.home(),
                Key::End => input.end(),
                _ => {}
            }
            if reopen {
                model.overlay = Some(Overlay::Revision { field, input });
            }
        }
        Overlay::Scope(mut chooser) => {
            let mut reopen = true;
            match key {
                Key::Esc => {
                    if chooser.input.is_some() {
                        chooser.input = None;
                        chooser.error = None;
                    } else {
                        reopen = false;
                    }
                }
                Key::Enter if chooser.input.is_some() => {
                    if let Some(input) = chooser.input.take() {
                        match RepoPath::new(input.value()) {
                            Ok(path) => {
                                let selection = Selection::new(vec![SelectionGroup::Path(path)]);
                                apply_selection(model, cmds, selection);
                                reopen = false;
                            }
                            Err(error) => {
                                chooser.error = Some(error.to_string());
                                chooser.input = Some(input);
                            }
                        }
                    }
                }
                Key::Enter => {
                    let area_count = chooser.area_count();
                    let cursor = chooser.cursor.min(area_count + 1);
                    if cursor == 0 {
                        apply_selection(model, cmds, Selection::all());
                        reopen = false;
                    } else if cursor <= area_count {
                        let area = chooser.areas.as_ref().and_then(|areas| {
                            let name = areas.names().nth(cursor - 1)?;
                            areas.get(name)
                        });
                        if let Some(area) = area.cloned() {
                            let selection = Selection::new(vec![SelectionGroup::Area(area)]);
                            apply_selection(model, cmds, selection);
                            reopen = false;
                        }
                    } else {
                        chooser.input = Some(TextInput::new(""));
                    }
                }
                Key::Up | Key::Char('k') if chooser.input.is_none() => {
                    chooser.cursor = chooser.cursor.saturating_sub(1);
                }
                Key::Down | Key::Char('j') if chooser.input.is_none() => {
                    let last = chooser.options().len().saturating_sub(1);
                    chooser.cursor = (chooser.cursor + 1).min(last);
                }
                Key::Char(character) if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.insert(character);
                    }
                }
                Key::Backspace if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.backspace();
                    }
                }
                Key::Delete if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.delete();
                    }
                }
                Key::Left if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.left();
                    }
                }
                Key::Right if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.right();
                    }
                }
                Key::Home if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.home();
                    }
                }
                Key::End if chooser.input.is_some() => {
                    if let Some(input) = &mut chooser.input {
                        input.end();
                    }
                }
                _ => {}
            }
            if reopen {
                model.overlay = Some(Overlay::Scope(chooser));
            }
        }
        Overlay::Mode { mut cursor } => {
            let mut reopen = true;
            let modes = available_modes();
            match key {
                Key::Esc => reopen = false,
                Key::Up | Key::Char('k') => cursor = cursor.saturating_sub(1),
                Key::Down | Key::Char('j') => cursor = (cursor + 1).min(modes.len() - 1),
                Key::Enter => {
                    reopen = false;
                    let mode = modes[cursor.min(modes.len() - 1)];
                    cmds.push(Cmd::Load {
                        request: model.request.with_mode(mode),
                    });
                }
                _ => {}
            }
            if reopen {
                model.overlay = Some(Overlay::Mode { cursor });
            }
        }
        Overlay::Palette(mut state) => {
            let mut reopen = true;
            match key {
                Key::Esc => reopen = false,
                Key::Up => state.cursor = state.cursor.saturating_sub(1),
                Key::Down => {
                    state.cursor = (state.cursor + 1).min(state.matches.len().saturating_sub(1));
                }
                Key::Enter => {
                    reopen = false;
                    if let Some(ranked) = state.matches.get(state.cursor) {
                        let entries = palette_entries(model);
                        if let Some((action, _, _)) = entries.get(ranked.index) {
                            let action = *action;
                            model.overlay = None;
                            apply_action(action, model, cmds);
                        }
                    }
                }
                Key::Char(character) => {
                    state.input.insert(character);
                    state.cursor = 0;
                    state.refresh(model);
                }
                Key::Backspace => {
                    state.input.backspace();
                    state.cursor = 0;
                    state.refresh(model);
                }
                _ => {}
            }
            if reopen {
                model.overlay = Some(Overlay::Palette(state));
            }
        }
        Overlay::Finder(mut state) => {
            let mut reopen = true;
            match key {
                Key::Esc => reopen = false,
                Key::Up => state.cursor = state.cursor.saturating_sub(1),
                Key::Down => {
                    state.cursor = (state.cursor + 1).min(state.matches.len().saturating_sub(1));
                }
                Key::Enter => {
                    reopen = false;
                    if let Some(ranked) = state.matches.get(state.cursor)
                        && let Some(path) = model.visible.get(ranked.index).cloned()
                    {
                        model.cursor = model.row_of_file(&path).unwrap_or(model.cursor);
                        model.selected = Some(path);
                        model.body_scroll = 0;
                        model.body_hscroll = 0;
                        model.overlay = None;
                        // Highlighting and search re-sync are deferred to
                        // [`settle`].
                    }
                }
                Key::Char(character) => {
                    state.input.insert(character);
                    state.cursor = 0;
                    state.refresh(model);
                }
                Key::Backspace => {
                    state.input.backspace();
                    state.cursor = 0;
                    state.refresh(model);
                }
                _ => {}
            }
            if reopen {
                model.overlay = Some(Overlay::Finder(state));
            }
        }
        Overlay::Search(mut state) => {
            let mut reopen = true;
            match key {
                Key::Esc => {
                    reopen = false;
                    model.search = None;
                }
                Key::Enter => {
                    reopen = false;
                    focus_current_match(model);
                }
                Key::Char(character) => {
                    state.input.insert(character);
                    let needle = state.input.value();
                    refresh_search(model, &needle);
                }
                Key::Backspace => {
                    state.input.backspace();
                    let needle = state.input.value();
                    refresh_search(model, &needle);
                }
                _ => {}
            }
            if reopen {
                model.overlay = Some(Overlay::Search(state));
            }
        }
    }
}

pub(crate) fn available_modes() -> [ProjectionMode; 2] {
    [ProjectionMode::Types, ProjectionMode::Signatures]
}

pub(crate) fn mode_label(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
    }
}

fn apply_selection(model: &mut Model, cmds: &mut Vec<Cmd>, selection: Selection) {
    model.overlay = None;
    cmds.push(Cmd::Load {
        request: model.request.with_selection(selection),
    });
}

fn current_revision(model: &Model) -> String {
    match &model.request {
        LoadRequest::Show { revision, .. } => revision.clone(),
        LoadRequest::Diff { .. } => "HEAD".to_owned(),
    }
}

fn current_base(model: &Model) -> String {
    match &model.request {
        LoadRequest::Diff { base, .. } => base.clone(),
        LoadRequest::Show { .. } => String::new(),
    }
}

fn current_target(model: &Model) -> String {
    match &model.request {
        LoadRequest::Diff { target, .. } => target.clone(),
        LoadRequest::Show { .. } => String::new(),
    }
}

fn cycle_focus(model: &mut Model, forward: bool) {
    let order = focus_order(model);
    let index = order
        .iter()
        .position(|pane| *pane == model.focus)
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % order.len()
    } else {
        (index + order.len() - 1) % order.len()
    };
    model.focus = order[next];
}

fn select_commit(model: &mut Model, index: usize, cmds: &mut Vec<Cmd>) {
    if model.request.diff_view() != Some(DiffView::Commits) || model.commits.is_empty() {
        return;
    }
    let index = index.min(model.commits.len() - 1);
    model.commit_target = (index != model.commit_cursor).then_some(index);
    if index != model.commit_cursor {
        cmds.push(Cmd::LoadStep {
            request: model.request.clone(),
            index,
            step: model.commits[index].clone(),
        });
    } else {
        model.pending = None;
    }
}

fn sync_commit_scroll(model: &mut Model) {
    let (_, content, _) = frame_areas(model.width, model.height);
    let layout = body_layout(content, model.tree_percent, true, true, model.focus);
    let Some(slot) = layout
        .slots
        .iter()
        .find(|slot| slot.pane == PaneSlot::Commits)
    else {
        return;
    };
    let inner = pane_block("", false, &model.theme, slot.edge).inner(slot.outer);
    model.commit_scroll = commit_offset(
        model.commit_cursor,
        model.commit_scroll,
        model.commits.len(),
        inner.height as usize,
    );
}

fn commits_key(key: Key, model: &mut Model, cmds: &mut Vec<Cmd>) {
    match key {
        Key::Down | Key::Char('j') => {
            select_commit(model, model.navigation_commit().saturating_add(1), cmds)
        }
        Key::Up | Key::Char('k') => {
            select_commit(model, model.navigation_commit().saturating_sub(1), cmds)
        }
        _ => {}
    }
}

fn tree_key(key: Key, model: &mut Model) {
    match key {
        Key::Down | Key::Char('j') => move_cursor(model, 1),
        Key::Up | Key::Char('k') => move_cursor(model, -1),
        Key::Right | Key::Char('l') => {
            let Some((path, expanded)) = directory_at_cursor(model) else {
                return;
            };
            if expanded {
                move_cursor(model, 1);
            } else {
                set_collapsed(model, path, false);
            }
        }
        Key::Left | Key::Char('h') => {
            let Some(row) = model.current_row() else {
                return;
            };
            let (path, expanded, is_dir) = match &row.kind {
                RowKind::Directory { path, expanded } => (path.clone(), *expanded, true),
                RowKind::File { path } => (path.clone(), false, false),
            };
            if is_dir && expanded {
                set_collapsed(model, path, true);
            } else {
                move_to_parent(model, &path);
            }
        }
        Key::Enter => {
            let Some(row) = model.current_row() else {
                return;
            };
            match row.kind.clone() {
                RowKind::Directory { path, expanded } => {
                    set_collapsed(model, path, expanded);
                }
                RowKind::File { path } => {
                    model.selected = Some(path);
                    // Highlighting and search re-sync are deferred to [`settle`].
                }
            }
        }
        _ => {}
    }
}

fn body_key(key: Key, model: &mut Model) {
    let show = matches!(model.content, Content::Show(_));
    match key {
        Key::Down | Key::Char('j') => {
            model.body_scroll = model.body_scroll.saturating_add(1).min(max_scroll(model));
        }
        Key::Up | Key::Char('k') => {
            model.body_scroll = model.body_scroll.saturating_sub(1);
        }
        // Diff MVP always wraps, so horizontal scrolling belongs to show only.
        Key::Right | Key::Char('l') if show => {
            model.body_hscroll = model.body_hscroll.saturating_add(1).min(max_hscroll(model));
        }
        Key::Left | Key::Char('h') if show => {
            model.body_hscroll = model.body_hscroll.saturating_sub(1);
        }
        _ => {}
    }
}

const MOUSE_SCROLL_STEP: i32 = 3;

// Routes a mouse event to the pane under the pointer.
//
// The pane under the pointer takes focus, so a click or wheel acts on exactly
// what the user points at. Mouse input is ignored while a modal overlay is
// open and when the terminal is too small to draw the browser.
fn handle_mouse(event: Mouse, model: &mut Model, cmds: &mut Vec<Cmd>) {
    if model.overlay.is_some() || model.width < SINGLE_PANE_MIN_WIDTH || model.height < MIN_HEIGHT {
        return;
    }

    let is_diff = matches!(model.content, Content::Diff(_));
    let (_, content, _) = frame_areas(model.width, model.height);
    let layout = body_layout(
        content,
        model.tree_percent,
        is_diff,
        model.request.diff_view() == Some(DiffView::Commits),
        model.focus,
    );
    let point = Position {
        x: event.column,
        y: event.row,
    };
    let Some(slot) = layout.slots.iter().find(|slot| slot.outer.contains(point)) else {
        return;
    };
    let inner = pane_block("", false, &model.theme, slot.edge).inner(slot.outer);

    match slot.pane {
        PaneSlot::Commits => {
            model.focus = Pane::Commits;
            match event.kind {
                MouseKind::Click => {
                    if inner.contains(point) {
                        let offset = commit_offset(
                            model.commit_cursor,
                            model.commit_scroll,
                            model.commits.len(),
                            inner.height as usize,
                        );
                        let index = offset + usize::from(event.row - inner.y);
                        if index < model.commits.len() {
                            select_commit(model, index, cmds);
                        }
                    }
                }
                MouseKind::ScrollUp => select_commit(
                    model,
                    model
                        .navigation_commit()
                        .saturating_sub(MOUSE_SCROLL_STEP as usize),
                    cmds,
                ),
                MouseKind::ScrollDown => select_commit(
                    model,
                    model
                        .navigation_commit()
                        .saturating_add(MOUSE_SCROLL_STEP as usize),
                    cmds,
                ),
            }
        }
        PaneSlot::Tree => {
            model.focus = Pane::Tree;
            match event.kind {
                MouseKind::Click => tree_click(model, inner, event.row),
                MouseKind::ScrollUp => move_cursor(model, -MOUSE_SCROLL_STEP),
                MouseKind::ScrollDown => move_cursor(model, MOUSE_SCROLL_STEP),
            }
        }
        PaneSlot::Show | PaneSlot::Old | PaneSlot::New => {
            model.focus = if is_diff { Pane::Diff } else { Pane::Body };
            match event.kind {
                MouseKind::Click => {}
                MouseKind::ScrollUp => {
                    model.body_scroll = model.body_scroll.saturating_sub(MOUSE_SCROLL_STEP as u16);
                }
                MouseKind::ScrollDown => {
                    let step = MOUSE_SCROLL_STEP as u16;
                    model.body_scroll = model
                        .body_scroll
                        .saturating_add(step)
                        .min(max_scroll(model));
                }
            }
        }
    }
}

fn tree_click(model: &mut Model, inner: Rect, row: u16) {
    if row < inner.y || row >= inner.y.saturating_add(inner.height) {
        return;
    }
    let offset = window_offset(model.cursor, model.rows.len(), inner.height as usize);
    let index = offset + usize::from(row - inner.y);
    let Some(kind) = model.rows.get(index).map(|row| row.kind.clone()) else {
        return;
    };
    model.cursor = index;
    match kind {
        RowKind::Directory { path, expanded } => set_collapsed(model, path, expanded),
        RowKind::File { .. } => sync_selected(model),
    }
}

fn move_cursor(model: &mut Model, delta: i32) {
    if model.rows.is_empty() {
        return;
    }
    let last = model.rows.len() - 1;
    let cursor = if delta < 0 {
        model.cursor.saturating_sub(delta.unsigned_abs() as usize)
    } else {
        (model.cursor + delta as usize).min(last)
    };
    model.cursor = cursor;
    sync_selected(model);
}

fn sync_selected(model: &mut Model) {
    let path = match model.current_row() {
        Some(TreeRow {
            kind: RowKind::File { path },
            ..
        }) => path.clone(),
        _ => return,
    };
    if model.selected.as_ref() != Some(&path) {
        model.body_scroll = 0;
        model.body_hscroll = 0;
    }
    model.selected = Some(path);
    // Highlighting and search re-sync are deferred to [`settle`].
}

fn directory_at_cursor(model: &Model) -> Option<(RepoPath, bool)> {
    match &model.current_row()?.kind {
        RowKind::Directory { path, expanded } => Some((path.clone(), *expanded)),
        RowKind::File { .. } => None,
    }
}

fn set_collapsed(model: &mut Model, path: RepoPath, collapsed: bool) {
    if collapsed {
        model.collapsed.insert(path.clone());
    } else {
        model.collapsed.remove(&path);
    }
    model.rows = Arc::from(build_rows(&model.visible, &model.collapsed));
    model.cursor = model.row_of_directory(&path).unwrap_or(0);
}

fn move_to_parent(model: &mut Model, path: &RepoPath) {
    let Some(separator) = path.as_bytes().iter().rposition(|&byte| byte == b'/') else {
        return;
    };
    let parent = RepoPath::new(&path.as_bytes()[..separator]).expect("parent of a normalized path");
    if let Some(index) = model.row_of_directory(&parent) {
        model.cursor = index;
    }
}

fn max_scroll(model: &Model) -> u16 {
    let count = match &model.content {
        Content::Show(_) => model.line_count(),
        Content::Diff(_) => model.derived.diff_rows.len(),
    };
    count.saturating_sub(1) as u16
}

fn max_hscroll(model: &Model) -> u16 {
    model.max_line_width().saturating_sub(1) as u16
}

fn clamp_view(model: &mut Model) {
    model.body_scroll = model.body_scroll.min(max_scroll(model));
    model.body_hscroll = model.body_hscroll.min(max_hscroll(model));
}

// Derives the visible tree rows from the visible files.
//
// Files arrive in raw path-byte order, so a directory's descendants form a
// contiguous block and each directory row is emitted exactly once, at the
// position of its first descendant. A collapsed directory emits only its own
// row and skips its entire subtree.
fn build_rows(visible: &[RepoPath], collapsed: &BTreeSet<RepoPath>) -> Vec<TreeRow> {
    let mut rows = Vec::new();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut index = 0;

    while index < visible.len() {
        let path = &visible[index];
        let components: Vec<&[u8]> = path.as_bytes().split(|&byte| byte == b'/').collect();
        let directories: Vec<Vec<u8>> = (1..components.len())
            .map(|depth| join_components(&components[..depth]))
            .collect();

        let common = stack
            .iter()
            .zip(&directories)
            .take_while(|(open, dir)| open == dir)
            .count();
        stack.truncate(common);

        let mut hidden = false;
        let mut depth = common;
        while depth < directories.len() {
            let directory_bytes = &directories[depth];
            let directory =
                RepoPath::new(directory_bytes.as_slice()).expect("a normalized directory path");
            let is_collapsed = collapsed.contains(&directory);
            rows.push(TreeRow {
                depth,
                label: component_label(components[depth]),
                kind: RowKind::Directory {
                    path: directory.clone(),
                    expanded: !is_collapsed,
                },
            });

            if is_collapsed {
                hidden = true;
                index += 1;
                while index < visible.len() && is_within(visible[index].as_bytes(), directory_bytes)
                {
                    index += 1;
                }
                break;
            }

            stack.push(directory_bytes.clone());
            depth += 1;
        }

        if hidden {
            continue;
        }

        rows.push(TreeRow {
            depth: directories.len(),
            label: component_label(components[components.len() - 1]),
            kind: RowKind::File { path: path.clone() },
        });
        index += 1;
    }

    rows
}

fn join_components(components: &[&[u8]]) -> Vec<u8> {
    let mut joined = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            joined.push(b'/');
        }
        joined.extend_from_slice(component);
    }
    joined
}

fn component_label(component: &[u8]) -> String {
    RepoPath::new(component).map_or_else(
        |_| String::from_utf8_lossy(component).into_owned(),
        |path| path.to_string(),
    )
}

fn is_within(path: &[u8], directory: &[u8]) -> bool {
    path.len() > directory.len() && path[directory.len()] == b'/' && path.starts_with(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Run;
    use crate::icons::IconStyle;
    use crate::view::geom::window_offset;
    use crate::view::text::clip_line;
    use crate::view::view;
    use base::{Area, ItemKind, ProjectedItem, SelectionError, SourceSpan, SupportedPath};
    use ratatui::style::Color;

    fn item(text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: "item".to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: "item".to_owned(),
            span: SourceSpan::new(0, 0, 0, 0, 0, 0),
            canonical_text: text.to_owned(),
        }
    }

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path = SupportedPath::new(RepoPath::new(path).expect("valid path"))
            .expect("test path is supported");
        if text.is_empty() {
            return ProjectedFile::try_new(path, Vec::new()).expect("valid fixture");
        }
        ProjectedFile::try_new(path, vec![item(text)]).expect("valid fixture")
    }

    fn show_request() -> LoadRequest {
        LoadRequest::Show {
            revision: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
        }
    }

    fn diff_request() -> LoadRequest {
        LoadRequest::Diff {
            base: "HEAD~1".to_owned(),
            target: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
            view: DiffView::Range,
        }
    }

    fn commit_steps(count: usize) -> Vec<CommitStep> {
        use git::{HashKind, ObjectId};
        (0..count)
            .map(|index| CommitStep {
                parent_id: ObjectId {
                    kind: HashKind::Sha1,
                    bytes: vec![index as u8; 20],
                },
                commit_id: ObjectId {
                    kind: HashKind::Sha1,
                    bytes: vec![(index + 1) as u8; 20],
                },
                subject: format!("Commit subject {index}"),
            })
            .collect()
    }

    fn commits_model(count: usize) -> Model {
        let mut model = diff_model(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        model.request = diff_request().with_diff_view(DiffView::Commits);
        model.commits = commit_steps(count).into();
        model.focus = Pane::Commits;
        model
    }

    #[test]
    fn empty_commit_history_installs_a_diff_and_focuses_commits() {
        let model = diff_model(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let request = diff_request().with_diff_view(DiffView::Commits);
        let (next, commands) = update(
            Msg::HistoryLoaded {
                request: request.clone(),
                result: Ok((Vec::new(), Content::Diff(Vec::new().into()))),
            },
            &model,
        );
        assert!(commands.is_empty());
        assert_eq!(next.request, request);
        assert_eq!(next.focus, Pane::Commits);
        assert!(next.commits.is_empty());
        assert!(next.visible.is_empty());
        assert!(next.selected.is_none());
        let (unchanged, commands) = update(Msg::Key(Key::Down), &next);
        assert!(commands.is_empty());
        assert_eq!(unchanged.commit_cursor, 0);
    }

    #[test]
    fn commit_picker_renders_rows_and_selected_revision_labels() {
        let model = commits_model(12);
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("Commits"));
        assert!(text.contains("Files"));
        assert!(text.contains("Commit sub"));
        assert!(text.contains("1/12"));
        assert!(text.contains("0000000..0101010"));

        let mut empty = commits_model(0);
        empty.content = Content::Diff(Vec::new().into());
        let text = buffer_text(&render(&empty, 100, 20));
        assert!(text.contains("No commits"));
    }

    #[test]
    fn commit_picker_mouse_uses_drawn_geometry_and_loads_selected_step() {
        let model = commits_model(12);
        let (clicked, commands) = update(Msg::Mouse(click(4, 3)), &model);
        assert_eq!(clicked.focus, Pane::Commits);
        assert!(matches!(
            commands.as_slice(),
            [Cmd::LoadStep { index: 1, .. }]
        ));

        let (wheeled, commands) = update(Msg::Mouse(scroll(4, 3, false)), &model);
        assert_eq!(wheeled.focus, Pane::Commits);
        assert!(matches!(
            commands.as_slice(),
            [Cmd::LoadStep { index: 3, .. }]
        ));

        let mut narrow = model.clone();
        narrow.width = 60;
        let (_, commands) = update(Msg::Mouse(click(4, 3)), &narrow);
        assert!(matches!(
            commands.as_slice(),
            [Cmd::LoadStep { index: 1, .. }]
        ));

        let (files, commands) = update(Msg::Mouse(click(4, 11)), &model);
        assert_eq!(files.focus, Pane::Tree);
        assert!(commands.is_empty());
    }

    #[test]
    fn batched_commit_navigation_loads_only_the_final_step_and_failure_keeps_the_displayed_step() {
        let mut model = commits_model(6);
        let displayed = model.content.clone();
        let displayed_labels = model.diff_revisions();
        let mut commands = Vec::new();
        for key in [Key::Down, Key::Char('j'), Key::Down] {
            let (next, produced) = update(Msg::Key(key), &model);
            model = next;
            commands.extend(produced);
        }
        assert_eq!(
            model.commit_cursor, 0,
            "the visible step is still the loaded one"
        );
        assert_eq!(model.commit_target, Some(3));
        assert_eq!(commands.len(), 3);
        coalesce_commit_loads(&mut model, &mut commands);
        assert!(matches!(
            commands.as_slice(),
            [Cmd::LoadStep { index: 3, .. }]
        ));
        assert_eq!(model.content, displayed);
        assert_eq!(model.diff_revisions(), displayed_labels);

        let (loaded, _) = update(
            Msg::StepLoaded {
                request: model.request.clone(),
                index: 3,
                result: Ok(Content::Diff(
                    vec![file_diff("a.rs", Some("a\n"), Some("new\n"))].into(),
                )),
            },
            &model,
        );
        assert_eq!(loaded.commit_cursor, 3);
        assert_eq!(loaded.commit_target, None);
        assert_ne!(loaded.diff_revisions(), displayed_labels);
        assert_eq!(loaded.selected, model.selected);

        let (failed, commands) = update(
            Msg::StepLoaded {
                request: model.request.clone(),
                index: 3,
                result: Err(Box::new(EngineError::Selection(
                    SelectionError::UnknownArea {
                        name: "missing".to_owned(),
                    },
                ))),
            },
            &model,
        );
        assert!(commands.is_empty());
        assert_eq!(failed.commit_cursor, 0);
        assert_eq!(failed.commit_target, None);
        assert_eq!(failed.content, displayed);
        assert_eq!(failed.diff_revisions(), displayed_labels);
        assert!(failed.diagnostic.is_some());
    }

    #[test]
    fn commit_navigation_back_to_loaded_step_cancels_batched_projection() {
        let mut model = commits_model(3);
        let mut commands = Vec::new();
        for key in [Key::Down, Key::Up] {
            let (next, produced) = update(Msg::Key(key), &model);
            model = next;
            commands.extend(produced);
        }
        coalesce_commit_loads(&mut model, &mut commands);
        assert!(commands.is_empty());
        assert!(model.pending.is_none());
        assert_eq!(model.commit_target, None);
    }

    #[test]
    fn palette_switches_between_range_and_commit_views() {
        let model = diff_model(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        assert!(
            palette_entries(&model)
                .iter()
                .any(|(action, ..)| *action == Action::SwitchToCommits)
        );
        let mut next = model.clone();
        let mut commands = Vec::new();
        apply_action(Action::SwitchToCommits, &mut next, &mut commands);
        assert_eq!(
            commands,
            vec![Cmd::Load {
                request: diff_request().with_diff_view(DiffView::Commits)
            }]
        );
        assert_eq!(
            next.request,
            diff_request(),
            "view changes after the load succeeds"
        );
    }

    fn file_diff(path: &str, old: Option<&str>, new: Option<&str>) -> FileDiff {
        match (old, new) {
            (None, Some(text)) => FileDiff::Added {
                new: projected(path, text),
            },
            (Some(text), None) => FileDiff::Deleted {
                old: projected(path, text),
            },
            (Some(old_text), Some(new_text)) => FileDiff::Modified {
                old: projected(path, old_text),
                new: projected(path, new_text),
            },
            (None, None) => panic!("a test diff needs at least one side"),
        }
    }

    fn model_with(files: Vec<ProjectedFile>) -> Model {
        let mut model = Model::new(
            "/repo".to_owned(),
            show_request(),
            "all".to_owned(),
            100,
            30,
            Theme::dark(),
            Icons::new(IconStyle::None),
        );
        model.install(Content::Show(files.into()), false);
        // Install defers selection work; settle so rendering tests see the
        // highlighted, laid-out model a frame would.
        settle(model)
    }

    fn diff_model(diffs: Vec<FileDiff>) -> Model {
        let mut model = Model::new(
            "/repo".to_owned(),
            diff_request(),
            "all".to_owned(),
            100,
            30,
            Theme::dark(),
            Icons::new(IconStyle::None),
        );
        model.install(Content::Diff(diffs.into()), false);
        settle(model)
    }

    // A show model with the selection work *not* settled, for deferral tests.
    fn unsettled_show(files: Vec<ProjectedFile>) -> Model {
        let mut model = Model::new(
            "/repo".to_owned(),
            show_request(),
            "all".to_owned(),
            100,
            30,
            Theme::dark(),
            Icons::new(IconStyle::None),
        );
        model.install(Content::Show(files.into()), false);
        model
    }

    fn two_files() -> Model {
        model_with(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")])
    }

    #[test]
    fn navigation_defers_highlighting_until_settle() {
        let model = unsettled_show(vec![
            projected("a.rs", "pub fn a();\n"),
            projected("b.rs", "pub fn b();\n"),
        ]);
        assert!(
            model.active_show_lines().is_none(),
            "install must not highlight"
        );

        let (moved, _) = update(Msg::Key(Key::Down), &model);
        assert!(
            moved.active_show_lines().is_none(),
            "update must not highlight"
        );

        let settled = settle(moved);
        assert!(
            settled.active_show_lines().is_some(),
            "settle highlights the final selection"
        );
    }

    #[test]
    fn settle_skips_highlighting_when_the_body_is_hidden() {
        let mut model = unsettled_show(vec![projected("a.rs", "pub fn a();\n")]);
        model.width = 60;
        model.focus = Pane::Tree;

        let settled = settle(model);
        assert!(
            settled.active_show_lines().is_none(),
            "a hidden body must not be highlighted"
        );
    }

    #[test]
    fn the_highlight_cache_is_bounded() {
        let mut cache: BoundedCache<usize> = BoundedCache::default();
        for index in 0..(HIGHLIGHT_CACHE_CAP + 10) {
            let path = RepoPath::new(format!("f{index}.rs")).expect("valid path");
            cache.insert(path, index);
        }

        assert_eq!(cache.order.len(), HIGHLIGHT_CACHE_CAP);
        assert_eq!(cache.entries.len(), HIGHLIGHT_CACHE_CAP);
        assert!(!cache.contains_key(&RepoPath::new("f0.rs").expect("valid path")));
        let newest = RepoPath::new(format!("f{}.rs", HIGHLIGHT_CACHE_CAP + 9)).expect("valid path");
        assert!(cache.contains_key(&newest));
    }

    #[test]
    fn mode_picker_defers_the_mode_change_until_selection() {
        let model = two_files();
        let (opened, cmds) = update(Msg::Key(Key::Char('m')), &model);
        assert!(cmds.is_empty());
        assert!(matches!(opened.overlay, Some(Overlay::Mode { .. })));

        let (down, _) = update(Msg::Key(Key::Down), &opened);
        let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);

        assert_eq!(model.mode, ProjectionMode::Types);
        assert_eq!(chosen.mode, ProjectionMode::Types, "mode changes on Loaded");
        assert_eq!(chosen.overlay, None);
        assert_eq!(
            cmds,
            vec![Cmd::Load {
                request: show_request().with_mode(ProjectionMode::Signatures),
            }]
        );
    }

    #[test]
    fn mode_picker_on_a_diff_keeps_the_diff_shape() {
        let model = diff_model(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let (opened, _) = update(Msg::Key(Key::Char('m')), &model);
        let (down, _) = update(Msg::Key(Key::Down), &opened);
        let (_, cmds) = update(Msg::Key(Key::Enter), &down);

        assert_eq!(
            cmds,
            vec![Cmd::Load {
                request: diff_request().with_mode(ProjectionMode::Signatures),
            }]
        );
    }

    #[test]
    fn the_mode_picker_lists_the_modes() {
        let mut model = two_files();
        model.overlay = Some(Overlay::Mode { cursor: 0 });
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("types"), "{text}");
        assert!(text.contains("signatures"), "{text}");
    }

    #[test]
    fn tree_resize_keys_step_clamp_and_reset() {
        let model = two_files();
        let (grow, _) = update(Msg::Key(Key::Char(']')), &model);
        assert_eq!(grow.tree_percent, TREE_DEFAULT_PERCENT + TREE_STEP);
        let (shrink, _) = update(Msg::Key(Key::Char('[')), &grow);
        assert_eq!(shrink.tree_percent, TREE_DEFAULT_PERCENT);

        let mut widest = model.clone();
        widest.tree_percent = TREE_MAX_PERCENT;
        let (widest, _) = update(Msg::Key(Key::Char(']')), &widest);
        assert_eq!(widest.tree_percent, TREE_MAX_PERCENT);

        let mut narrowest = model.clone();
        narrowest.tree_percent = TREE_MIN_PERCENT;
        let (narrowest, _) = update(Msg::Key(Key::Char('[')), &narrowest);
        assert_eq!(narrowest.tree_percent, TREE_MIN_PERCENT);

        let (reset, _) = update(Msg::Key(Key::Char('\\')), &grow);
        assert_eq!(reset.tree_percent, TREE_DEFAULT_PERCENT);
    }

    #[test]
    fn a_wider_tree_still_leaves_a_usable_body() {
        let model = model_with(vec![projected("a.rs", "pub fn a();\n")]);
        let mut wide_tree = model.clone();
        wide_tree.tree_percent = TREE_MAX_PERCENT;
        let text = buffer_text(&render(&wide_tree, 100, 20));
        assert!(text.contains("Files"), "{text}");
        assert!(text.contains("pub fn a();"), "{text}");
    }

    #[test]
    fn the_status_bar_shows_mode_scope_and_hints() {
        let model = model_with(vec![projected("a.rs", "x\n")]);
        let buffer = render(&model, 120, 20);
        let header = buffer_row(&buffer, 0);
        let footer = buffer_row(&buffer, 19);
        assert!(header.contains("types"), "{header}");
        assert!(header.contains("HEAD"), "{header}");
        assert!(header.contains("scope:"), "{header}");
        assert!(!header.contains("a.rs"), "{header}");
        assert!(footer.contains("a.rs"), "{footer}");
        assert!(footer.contains("1 lines"), "{footer}");
        assert!(footer.contains("help"), "{footer}");
    }

    #[test]
    fn narrow_chrome_keeps_context_and_the_selected_path() {
        let model = model_with(vec![projected("src/very/deep/file.rs", "x\n")]);
        let buffer = render(&model, 60, 20);
        let header = buffer_row(&buffer, 0);
        let footer = buffer_row(&buffer, 19);
        assert!(header.contains("types"), "{header}");
        assert!(header.contains("HEAD"), "{header}");
        assert!(footer.contains("file.rs"), "{footer}");
        assert!(footer.contains("lines"), "{footer}");
        assert!(footer.contains("help"), "{footer}");
        assert_eq!(
            buffer_text(&buffer)
                .matches("src/very/deep/file.rs")
                .count(),
            1
        );
    }

    #[test]
    fn the_status_bar_counts_diff_changes() {
        let model = diff_model(vec![file_diff(
            "a.rs",
            Some("one\ntwo\n"),
            Some("one\n2\n"),
        )]);
        let text = buffer_text(&render(&model, 120, 20));
        assert!(text.contains("+1"), "{text}");
        assert!(text.contains("−1"), "{text}");
    }

    #[test]
    fn a_diagnostic_renders_as_a_toast() {
        let mut model = two_files();
        model.diagnostic = Some("boom".to_owned());
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("boom"), "{text}");
    }

    #[test]
    fn a_load_marks_the_model_busy_until_it_completes() {
        let model = two_files();
        assert!(model.pending.is_none(), "an installed model is idle");

        let (opened, cmds) = update(Msg::Key(Key::Char('m')), &model);
        assert!(cmds.is_empty(), "opening the picker emits no effect");
        let (down, _) = update(Msg::Key(Key::Down), &opened);
        let (loading, cmds) = update(Msg::Key(Key::Enter), &down);
        assert!(!cmds.is_empty());
        assert!(loading.pending.is_some(), "an emitted load is busy");

        let (done, _) = update(
            Msg::Loaded {
                request: show_request().with_mode(ProjectionMode::Signatures),
                result: Ok(Content::Show(
                    vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")].into(),
                )),
            },
            &loading,
        );
        assert!(done.pending.is_none(), "completion clears the busy state");
    }

    #[test]
    fn a_tick_advances_the_spinner_and_expires_a_diagnostic() {
        let mut model = two_files();
        let error = EngineError::Selection(SelectionError::UnknownArea {
            name: "x".to_owned(),
        });
        let (failed, _) = update(
            Msg::Loaded {
                request: show_request(),
                result: Err(Box::new(error)),
            },
            &model,
        );
        assert!(failed.diagnostic.is_some());

        model = failed;
        let before = model.spinner;
        for _ in 0..DIAGNOSTIC_TICKS {
            let (next, _) = update(Msg::Tick, &model);
            model = next;
        }
        assert_eq!(model.spinner, before.wrapping_add(DIAGNOSTIC_TICKS));
        assert!(model.diagnostic.is_none(), "the toast expires on its own");
    }

    #[test]
    fn a_busy_model_shows_a_spinner_in_the_footer() {
        let mut model = two_files();
        model.pending = Some(show_request());
        let text = buffer_text(&render(&model, 120, 20));
        assert!(text.contains("projecting"), "{text}");
    }

    #[test]
    fn ctrl_p_opens_the_palette_and_filters_to_an_action() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::CtrlP), &model);
        assert!(matches!(opened.overlay, Some(Overlay::Palette(_))));

        let mut state = opened;
        for character in "help".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &state);
            state = next;
        }
        let Some(Overlay::Palette(palette)) = &state.overlay else {
            panic!("expected the palette");
        };
        let entries = palette_entries(&state);
        assert_eq!(entries[palette.matches[0].index].0, Action::Help);

        let (applied, _) = update(Msg::Key(Key::Enter), &state);
        assert_eq!(applied.overlay, Some(Overlay::Help));
    }

    #[test]
    fn the_finder_selects_a_file() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::CtrlF), &model);
        assert!(matches!(opened.overlay, Some(Overlay::Finder(_))));

        let (typed, _) = update(Msg::Key(Key::Char('b')), &opened);
        let (chosen, _) = update(Msg::Key(Key::Enter), &typed);
        assert_eq!(
            chosen.selected.as_ref().map(ToString::to_string),
            Some("b.rs".to_owned())
        );
        assert_eq!(chosen.overlay, None);
    }

    #[test]
    fn search_finds_matches_and_steps_them() {
        let model = model_with(vec![projected("a.rs", "alpha\nbeta\nalpha again\n")]);
        let (opened, _) = update(Msg::Key(Key::Char('/')), &model);
        assert!(matches!(opened.overlay, Some(Overlay::Search(_))));

        let mut state = opened;
        for character in "alpha".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &state);
            state = next;
        }
        assert_eq!(state.search.as_ref().expect("live search").matches.len(), 2);

        let (committed, _) = update(Msg::Key(Key::Enter), &state);
        assert_eq!(committed.overlay, None);
        let (stepped, _) = update(Msg::Key(Key::Char('n')), &committed);
        assert_eq!(stepped.search.as_ref().unwrap().cursor, 1);
        let (back, _) = update(Msg::Key(Key::Char('N')), &stepped);
        assert_eq!(back.search.as_ref().unwrap().cursor, 0);
    }

    #[test]
    fn switching_files_recomputes_a_committed_search() {
        let mut model = model_with(vec![
            projected("a.rs", "alpha\n"),
            projected("b.rs", "beta\nalpha\n"),
        ]);
        model.search = Some(Search {
            needle: "alpha".to_owned(),
            matches: vec![SearchMatch {
                side: SearchSide::Show,
                line: 1,
                start: 0,
                end: 5,
            }],
            cursor: 0,
        });

        model.selected = Some(RepoPath::new("b.rs").unwrap());
        model.resync_search();

        let search = model.search.as_ref().expect("search");
        assert_eq!(search.matches.len(), 1);
        assert_eq!(search.matches[0].line, 2, "the match moved to the new file");
    }

    #[test]
    fn q_types_into_a_search_input_instead_of_quitting() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('/')), &model);
        let (typed, _) = update(Msg::Key(Key::Char('q')), &opened);
        assert!(!typed.quit);
        match &typed.overlay {
            Some(Overlay::Search(state)) => assert_eq!(state.input.text, "q"),
            other => panic!("expected a search overlay, got {other:?}"),
        }
    }

    #[test]
    fn the_palette_and_finder_render_their_lists() {
        let mut model = two_files();
        model.overlay = Some(Overlay::Palette(PaletteState::new(&model)));
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("commands"), "{text}");
        assert!(text.contains("Switch Types"), "{text}");

        let mut model = two_files();
        model.overlay = Some(Overlay::Finder(FinderState::new(&model)));
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("find file"), "{text}");
        assert!(text.contains("a.rs"), "{text}");
    }

    #[test]
    fn page_keys_move_the_cursor_in_the_tree_and_scroll_the_body() {
        let model = model_with(vec![
            projected("a.rs", "one\ntwo\nthree\nfour\nfive\n"),
            projected("b.rs", "x\n"),
        ]);
        let (down, _) = update(Msg::Key(Key::PageDown), &model);
        assert!(down.cursor > model.cursor);

        let mut body = model.clone();
        body.focus = Pane::Body;
        let (scrolled, _) = update(Msg::Key(Key::PageDown), &body);
        assert!(scrolled.body_scroll > 0);
    }

    // -----------------------------------------------------------------------
    // mouse
    // -----------------------------------------------------------------------

    fn click(column: u16, row: u16) -> Mouse {
        Mouse {
            column,
            row,
            kind: MouseKind::Click,
        }
    }

    fn scroll(column: u16, row: u16, up: bool) -> Mouse {
        Mouse {
            column,
            row,
            kind: if up {
                MouseKind::ScrollUp
            } else {
                MouseKind::ScrollDown
            },
        }
    }

    // The tree's inner origin for the 100×30 models below (one border and one
    // pad column on the left, one border row on top).
    const TREE_X: u16 = 2;
    const TREE_Y: u16 = 2;
    const CONTENT_X: u16 = 50;

    #[test]
    fn clicking_a_file_selects_it() {
        let model = two_files();
        let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &model);
        assert_eq!(
            next.selected.as_ref().map(ToString::to_string),
            Some("b.rs".to_owned())
        );
        assert_eq!(next.cursor, 1);
        assert_eq!(next.focus, Pane::Tree);
    }

    #[test]
    fn clicking_a_directory_folds_it() {
        let model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
        ]);
        let (folded, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &model);

        let labels: Vec<&str> = folded.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["a.rs", "src"]);
    }

    #[test]
    fn the_wheel_moves_the_tree_cursor_and_selects_as_it_passes() {
        let model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("b.rs", "b\n"),
            projected("c.rs", "c\n"),
        ]);
        let (moved, _) = update(Msg::Mouse(scroll(TREE_X, TREE_Y, false)), &model);
        assert_eq!(moved.focus, Pane::Tree);
        assert_eq!(moved.cursor, 2, "the wheel step clamps to the last row");
        assert_eq!(
            moved.selected.as_ref().map(ToString::to_string),
            Some("c.rs".to_owned())
        );
    }

    #[test]
    fn the_wheel_scrolls_the_content_and_focuses_it() {
        let model = model_with(vec![projected("a.rs", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")]);
        let (down, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &model);
        assert_eq!(down.focus, Pane::Body);
        assert_eq!(down.body_scroll, 3);

        let (up, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, true)), &down);
        assert_eq!(up.body_scroll, 0);
    }

    #[test]
    fn the_wheel_scrolls_a_diff_and_clicking_it_focuses() {
        let model = diff_model(vec![file_diff(
            "a.rs",
            Some("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n"),
            Some("1\n2\n3\n4\n5\n6\n7\n8\n9\nX\n"),
        )]);
        let (scrolled, _) = update(Msg::Mouse(scroll(CONTENT_X, 5, false)), &model);
        assert_eq!(scrolled.focus, Pane::Diff);
        assert!(scrolled.body_scroll > 0);

        let (clicked, _) = update(Msg::Mouse(click(CONTENT_X, 5)), &model);
        assert_eq!(clicked.focus, Pane::Diff);
        assert_eq!(clicked.body_scroll, 0);
    }

    #[test]
    fn mouse_is_ignored_while_an_overlay_is_open() {
        let mut model = two_files();
        model.overlay = Some(Overlay::Help);
        let (next, _) = update(Msg::Mouse(click(TREE_X, TREE_Y + 1)), &model);
        assert_eq!(next.cursor, model.cursor);
        assert_eq!(next.selected, model.selected);
    }

    #[test]
    fn a_click_on_the_header_or_a_divider_does_nothing() {
        let model = two_files();

        let (header, _) = update(Msg::Mouse(click(5, 0)), &model);
        assert_eq!(header.cursor, model.cursor);
        assert_eq!(header.selected, model.selected);

        // The divider sits in the one column between the two panes.
        let mut body_focused = model.clone();
        body_focused.focus = Pane::Body;
        let (divider, _) = update(Msg::Mouse(click(29, 5)), &body_focused);
        assert_eq!(divider.focus, Pane::Body, "the divider is not a pane");
        assert_eq!(divider.selected, model.selected);
    }

    #[test]
    fn a_loaded_result_installs_files_and_preserves_the_selection() {
        let model = two_files();
        let (next, _) = update(
            Msg::Loaded {
                request: show_request().with_mode(ProjectionMode::Signatures),
                result: Ok(Content::Show(
                    vec![projected("a.rs", "A\n"), projected("b.rs", "B\n")].into(),
                )),
            },
            &model,
        );

        assert_eq!(next.mode, ProjectionMode::Signatures);
        assert_eq!(
            next.selected.as_ref().map(ToString::to_string),
            Some("a.rs".to_owned())
        );
        assert_eq!(next.active_text(), Some("A\n"));
    }

    #[test]
    fn a_reload_that_drops_the_selected_file_picks_the_nearest_visible_one() {
        let mut model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("b.rs", "b\n"),
            projected("c.rs", "c\n"),
        ]);
        model.cursor = model.row_of_file(&RepoPath::new("b.rs").unwrap()).unwrap();
        model.selected = Some(RepoPath::new("b.rs").unwrap());

        let (next, _) = update(
            Msg::Loaded {
                request: show_request(),
                result: Ok(Content::Show(
                    vec![projected("a.rs", "a\n"), projected("c.rs", "c\n")].into(),
                )),
            },
            &model,
        );

        assert_eq!(
            next.selected.as_ref().map(ToString::to_string),
            Some("c.rs".to_owned())
        );
    }

    #[test]
    fn a_failed_reload_keeps_the_last_model_and_shows_a_diagnostic() {
        let model = two_files();
        let error = EngineError::Selection(SelectionError::UnknownArea {
            name: "x".to_owned(),
        });
        let (next, _) = update(
            Msg::Loaded {
                request: show_request().with_mode(ProjectionMode::Signatures),
                result: Err(Box::new(error)),
            },
            &model,
        );

        assert_eq!(next.mode, ProjectionMode::Types, "the mode must not change");
        assert!(matches!(next.content, Content::Show(ref files) if files.len() == 2));
        assert!(next.diagnostic.is_some());
        assert!(next.selected.is_some());
    }

    #[test]
    fn empty_projections_are_hidden_from_the_tree() {
        let model = model_with(vec![projected("empty.rs", ""), projected("full.rs", "x\n")]);

        let labels: Vec<&str> = model.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["full.rs"]);
        assert!(!model.visible.contains(&RepoPath::new("empty.rs").unwrap()));
    }

    #[test]
    fn a_diff_tree_contains_only_changed_paths() {
        let model = diff_model(vec![
            file_diff("src/a.rs", Some("a\n"), Some("A\n")),
            file_diff("src/b.rs", None, Some("b\n")),
        ]);

        let labels: Vec<&str> = model.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["src", "a.rs", "b.rs"]);
    }

    #[test]
    fn tree_rows_follow_raw_path_order_and_are_nested() {
        let model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
            projected("src/main.rs", "m\n"),
        ]);

        let labels: Vec<&str> = model.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
        assert_eq!(model.rows[0].depth, 0);
        assert_eq!(model.rows[1].depth, 0);
        assert_eq!(model.rows[2].depth, 1);
    }

    #[test]
    fn collapsing_a_directory_hides_its_children_and_expanding_restores_them() {
        let mut model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
            projected("src/main.rs", "m\n"),
        ]);
        model.cursor = 1;
        assert!(matches!(
            model.current_row().unwrap().kind,
            RowKind::Directory { .. }
        ));

        let (collapsed, _) = update(Msg::Key(Key::Enter), &model);
        let labels: Vec<&str> = collapsed
            .rows
            .iter()
            .map(|row| row.label.as_str())
            .collect();
        assert_eq!(labels, vec!["a.rs", "src"]);
        assert!(matches!(
            collapsed.rows[1].kind,
            RowKind::Directory {
                expanded: false,
                ..
            }
        ));

        let (expanded, _) = update(Msg::Key(Key::Right), &collapsed);
        let labels: Vec<&str> = expanded.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
    }

    #[test]
    fn left_on_a_directory_row_folds_it() {
        let mut model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("src/lib.rs", "l\n"),
        ]);
        model.cursor = 1;

        let (folded, _) = update(Msg::Key(Key::Left), &model);
        assert!(matches!(
            folded.rows[1].kind,
            RowKind::Directory {
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn body_scrolling_is_clamped_to_the_content() {
        let mut model = model_with(vec![projected("a.rs", "one\ntwo\nthree\n")]);
        model.focus = Pane::Body;

        for _ in 0..10 {
            let (next, _) = update(Msg::Key(Key::Char('j')), &model);
            model = next;
        }
        assert_eq!(
            model.body_scroll, 2,
            "three lines allow scrolling to index 2"
        );

        for _ in 0..10 {
            let (next, _) = update(Msg::Key(Key::Char('k')), &model);
            model = next;
        }
        assert_eq!(model.body_scroll, 0);
    }

    #[test]
    fn tab_toggles_the_tree_and_both_diff_panes() {
        let diff = diff_model(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))]);
        let (step, _) = update(Msg::Key(Key::Tab), &diff);
        assert_eq!(step.focus, Pane::Diff, "the diff panes are one focus unit");
        let (step, _) = update(Msg::Key(Key::Tab), &step);
        assert_eq!(step.focus, Pane::Tree);
        let (step, _) = update(Msg::Key(Key::BackTab), &step);
        assert_eq!(step.focus, Pane::Diff);

        let show = two_files();
        let (step, _) = update(Msg::Key(Key::Tab), &show);
        assert_eq!(step.focus, Pane::Body);
        let (step, _) = update(Msg::Key(Key::BackTab), &step);
        assert_eq!(step.focus, Pane::Tree);
    }

    #[test]
    fn a_narrow_diff_stacks_old_over_new_when_focused() {
        let mut model = diff_model(vec![file_diff(
            "a.rs",
            Some("old line\n"),
            Some("new line\n"),
        )]);
        model.focus = Pane::Diff;
        let text = buffer_text(&render(&model, 60, 20));
        assert!(text.contains("old line"), "{text}");
        assert!(text.contains("new line"), "{text}");
    }

    #[test]
    fn help_toggles_and_swallows_navigation() {
        let model = two_files();
        let (help, _) = update(Msg::Key(Key::Char('?')), &model);
        assert_eq!(help.overlay, Some(Overlay::Help));

        let (still, _) = update(Msg::Key(Key::Char('j')), &help);
        assert_eq!(still.cursor, help.cursor);
        assert_eq!(still.overlay, Some(Overlay::Help));

        let (closed, _) = update(Msg::Key(Key::Esc), &help);
        assert_eq!(closed.overlay, None);
    }

    #[test]
    fn text_input_edits_on_char_boundaries() {
        let mut input = TextInput::new("héllo");
        input.backspace();
        assert_eq!(input.text, "héll");
        input.home();
        input.delete();
        assert_eq!(input.text, "éll");
        input.insert('x');
        assert_eq!(input.text, "xéll");
        assert_eq!(input.cursor, 1);
        input.right();
        assert_eq!(input.cursor, 3);
        input.end();
        assert_eq!(input.cursor, input.text.len());
    }

    #[test]
    fn revision_prompt_prefills_the_current_revision() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('r')), &model);
        match opened.overlay {
            Some(Overlay::Revision { field, input }) => {
                assert_eq!(field, RevisionField::Show);
                assert_eq!(input.text, "HEAD");
                assert_eq!(input.cursor, 4);
            }
            other => panic!("expected a revision overlay, got {other:?}"),
        }
    }

    #[test]
    fn confirming_a_revision_emits_a_load_with_that_revision() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('r')), &model);
        let mut state = opened;
        for _ in 0.."HEAD".len() {
            let (next, _) = update(Msg::Key(Key::Backspace), &state);
            state = next;
        }
        for character in "HEAD~2".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &state);
            state = next;
        }

        let (applied, cmds) = update(Msg::Key(Key::Enter), &state);
        assert_eq!(applied.overlay, None);
        assert_eq!(
            cmds,
            vec![Cmd::Load {
                request: show_request().with_revision("HEAD~2".to_owned()),
            }]
        );
    }

    #[test]
    fn escaping_a_revision_prompt_emits_nothing() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('r')), &model);
        let (closed, cmds) = update(Msg::Key(Key::Esc), &opened);
        assert_eq!(closed.overlay, None);
        assert!(cmds.is_empty());
    }

    #[test]
    fn diff_revision_keys_target_base_and_target_fields() {
        let model = diff_model(vec![file_diff("a.rs", Some("a\n"), Some("b\n"))]);
        let (base, _) = update(Msg::Key(Key::Char('b')), &model);
        assert!(matches!(
            base.overlay,
            Some(Overlay::Revision {
                field: RevisionField::Base,
                ..
            })
        ));
        let (target, _) = update(Msg::Key(Key::Char('t')), &model);
        assert!(matches!(
            target.overlay,
            Some(Overlay::Revision {
                field: RevisionField::Target,
                ..
            })
        ));
    }

    #[test]
    fn the_revision_prompt_renders_its_label_and_text() {
        let mut model = two_files();
        model.overlay = Some(Overlay::Revision {
            field: RevisionField::Show,
            input: TextInput::new("HEAD~2"),
        });
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("revision"), "{text}");
        assert!(text.contains("HEAD~2"), "{text}");
    }

    fn area_set() -> AreaSet {
        AreaSet::new([
            Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
            Area::new("web", [RepoPath::new("src/web").unwrap()]).unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn scope_key_opens_the_chooser_and_requests_areas() {
        let model = two_files();
        let (opened, cmds) = update(Msg::Key(Key::Char('s')), &model);
        assert!(matches!(opened.overlay, Some(Overlay::Scope(_))));
        assert_eq!(cmds, vec![Cmd::LoadAreas]);
    }

    #[test]
    fn choosing_all_from_the_chooser_loads_everything() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &model);
        let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);

        let (chosen, cmds) = update(Msg::Key(Key::Enter), &loaded);
        assert_eq!(chosen.overlay, None);
        assert_eq!(
            cmds,
            vec![Cmd::Load {
                request: show_request().with_selection(Selection::all()),
            }]
        );
    }

    #[test]
    fn selecting_an_area_loads_that_area() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &model);
        let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);
        let (down, _) = update(Msg::Key(Key::Down), &loaded);

        let (chosen, cmds) = update(Msg::Key(Key::Enter), &down);
        assert_eq!(chosen.overlay, None);
        let expected = Selection::new(vec![SelectionGroup::Area(
            Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
        )]);
        assert_eq!(
            cmds,
            vec![Cmd::Load {
                request: show_request().with_selection(expected),
            }]
        );
    }

    #[test]
    fn a_config_error_keeps_the_chooser_open_with_a_message() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &model);
        let error = EngineError::Selection(SelectionError::UnknownArea {
            name: "x".to_owned(),
        });
        let (loaded, _) = update(Msg::AreasLoaded(Err(Box::new(error))), &opened);

        match loaded.overlay {
            Some(Overlay::Scope(chooser)) => assert!(chooser.error.is_some()),
            other => panic!("expected a scope chooser, got {other:?}"),
        }
    }

    #[test]
    fn literal_path_entry_builds_a_path_selection() {
        let model = two_files();
        let (opened, _) = update(Msg::Key(Key::Char('s')), &model);
        let (loaded, _) = update(Msg::AreasLoaded(Ok(area_set())), &opened);

        let mut state = loaded;
        for _ in 0..3 {
            let (next, _) = update(Msg::Key(Key::Down), &state);
            state = next;
        }
        let (input, _) = update(Msg::Key(Key::Enter), &state);
        let mut typed = input;
        for character in "src/lib.rs".chars() {
            let (next, _) = update(Msg::Key(Key::Char(character)), &typed);
            typed = next;
        }

        let (chosen, cmds) = update(Msg::Key(Key::Enter), &typed);
        assert_eq!(chosen.overlay, None);
        let expected = Selection::new(vec![SelectionGroup::Path(
            RepoPath::new("src/lib.rs").unwrap(),
        )]);
        assert_eq!(
            cmds,
            vec![Cmd::Load {
                request: show_request().with_selection(expected),
            }]
        );
    }

    #[test]
    fn clip_line_slices_by_display_width() {
        assert_eq!(clip_line("abcdef", 2, 3), "cde");
        assert_eq!(clip_line("abcdef", 0, 0), "");
        assert_eq!(clip_line("abcdef", 10, 3), "");
        assert_eq!(clip_line("a\tb", 0, 10), "a    b");
        assert_eq!(clip_line("日本", 0, 3), "日");
        assert_eq!(clip_line("日本", 1, 4), "本");
    }

    #[test]
    fn window_offset_keeps_the_cursor_visible() {
        assert_eq!(window_offset(0, 100, 10), 0);
        assert_eq!(window_offset(9, 100, 10), 0);
        assert_eq!(window_offset(10, 100, 10), 1);
        assert_eq!(window_offset(0, 3, 10), 0);
        assert_eq!(window_offset(5, 100, 0), 0);
    }

    // -----------------------------------------------------------------------
    // diff layout
    // -----------------------------------------------------------------------

    fn runs_text(runs: &[Run]) -> String {
        runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn layout_diff_pads_the_shorter_side_and_marks_continuations() {
        let diff = file_diff(
            "a.rs",
            Some(&format!("{}\nsecond\n", "x".repeat(25))),
            Some("short\nsecond\n"),
        );
        let all = layout_diff(&diff, 10, 10, &[], &[], &Theme::dark());
        let rows: Vec<&VisualRow> = all
            .iter()
            .filter(|row| row.kind != VisualRowKind::Hunk)
            .collect();

        assert_eq!(rows.len(), 4, "3 for the wrapped row + 1 for the rest");
        assert!(!rows[0].continuation);
        assert!(rows[1].continuation && rows[2].continuation);
        assert!(rows[0].old_number.is_some());
        assert!(
            rows[1].old_number.is_none(),
            "continuations carry no number"
        );
        // The new side is padded so the second logical row stays aligned.
        assert_eq!(runs_text(&rows[1].new_runs), "");
        assert!(rows[3].old_number.is_some() && rows[3].new_number.is_some());
    }

    #[test]
    fn layout_diff_handles_an_added_file_with_a_missing_old_side() {
        let diff = file_diff("a.rs", None, Some("one\ntwo\n"));
        let all = layout_diff(&diff, 20, 20, &[], &[], &Theme::dark());
        let rows: Vec<&VisualRow> = all
            .iter()
            .filter(|row| row.kind != VisualRowKind::Hunk)
            .collect();

        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.kind == VisualRowKind::Diff(DiffRowKind::Add))
        );
        assert!(rows.iter().all(|row| row.old_number.is_none()));
        assert_eq!(rows[0].new_number, Some(1));
        assert_eq!(rows[1].new_number, Some(2));
    }

    #[test]
    fn layout_diff_handles_a_deleted_file() {
        let diff = file_diff("a.rs", Some("one\n"), None);
        let all = layout_diff(&diff, 20, 20, &[], &[], &Theme::dark());
        let rows: Vec<&VisualRow> = all
            .iter()
            .filter(|row| row.kind != VisualRowKind::Hunk)
            .collect();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, VisualRowKind::Diff(DiffRowKind::Delete));
        assert!(rows[0].new_number.is_none());
    }

    #[test]
    fn layout_diff_emits_a_hunk_header_with_line_ranges() {
        let diff = file_diff("a.rs", Some("one\ntwo\n"), Some("one\n2\n"));
        let rows = layout_diff(&diff, 20, 20, &[], &[], &Theme::dark());

        assert_eq!(rows[0].kind, VisualRowKind::Hunk);
        assert_eq!(runs_text(&rows[0].old_runs), "@@ -1,2 +1,2 @@");
    }

    #[test]
    fn layout_diff_collapses_long_unchanged_runs_between_hunks() {
        let old = (0..40)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut changed: Vec<String> = (0..40).map(|index| format!("line {index}")).collect();
        changed[0] = "changed".to_owned();
        changed[39] = "changed too".to_owned();
        let new = changed.join("\n") + "\n";

        let diff = file_diff("a.rs", Some(&old), Some(&new));
        let rows = layout_diff(&diff, 20, 20, &[], &[], &Theme::dark());

        let hunks = rows
            .iter()
            .filter(|row| row.kind == VisualRowKind::Hunk)
            .count();
        assert_eq!(hunks, 2, "two separated changes produce two hunks");
        assert!(rows.len() < 40, "the middle context is collapsed");
    }

    #[test]
    fn layout_diff_marks_the_collapsed_gap() {
        let old = (0..40)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let mut changed: Vec<String> = (0..40).map(|index| format!("line {index}")).collect();
        changed[0] = "changed".to_owned();
        changed[39] = "changed too".to_owned();
        let new = changed.join("\n") + "\n";

        let diff = file_diff("a.rs", Some(&old), Some(&new));
        let rows = layout_diff(&diff, 20, 20, &[], &[], &Theme::dark());

        let hidden: Vec<usize> = rows
            .iter()
            .filter_map(|row| match row.kind {
                VisualRowKind::Collapse(count) => Some(count),
                _ => None,
            })
            .collect();
        assert_eq!(hidden.len(), 1, "one gap between the two hunks");
        assert!(hidden[0] > 0);
    }

    // -----------------------------------------------------------------------
    // rendering
    // -----------------------------------------------------------------------

    fn render(model: &Model, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal.draw(|frame| view(model, frame)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut text = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }
        text
    }

    fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .filter_map(|x| buffer.cell((x, y)))
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn a_wide_terminal_shows_the_tree_and_the_projection() {
        let model = model_with(vec![projected("a.rs", "pub fn a();\n")]);
        let text = buffer_text(&render(&model, 100, 20));

        assert!(text.contains("Files"), "tree pane missing: {text}");
        assert!(text.contains("pub fn a();"), "body missing: {text}");
    }

    #[test]
    fn the_tree_keeps_a_quiet_selected_row_when_the_body_has_focus() {
        use crate::theme::{Capability, Flavor};

        for flavor in [Flavor::Dark, Flavor::Light] {
            let mut model = two_files();
            model.theme = Theme::new(flavor, Capability::TrueColor);

            let focused = render(&model, 100, 20);
            let focused_row = (0..focused.area.height)
                .find(|&y| {
                    focused
                        .cell((2, y))
                        .is_some_and(|cell| cell.symbol() == "▌")
                })
                .expect("selected file row");
            assert_eq!(focused.cell((2, focused_row)).unwrap().symbol(), "▌");
            assert_eq!(
                focused.cell((3, focused_row)).unwrap().bg,
                model.theme.color(model.theme.palette.selection_bg)
            );

            model.focus = Pane::Body;
            let unfocused = render(&model, 100, 20);
            assert_eq!(unfocused.cell((2, focused_row)).unwrap().symbol(), "▏");
            assert_eq!(
                unfocused.cell((3, focused_row)).unwrap().bg,
                model.theme.color(model.theme.palette.surface_alt)
            );
            assert_eq!(
                unfocused.cell((3, focused_row + 1)).unwrap().bg,
                model.theme.color(model.theme.palette.bg)
            );
        }

        let mut model = two_files();
        model.theme = Theme::new(Flavor::Dark, Capability::NoColor);
        model.focus = Pane::Body;
        assert!(buffer_text(&render(&model, 100, 20)).contains("▏a.rs"));
    }

    #[test]
    fn nerd_icons_render_and_leave_labels_intact() {
        let build = || {
            model_with(vec![
                projected("src/main.rs", "x\n"),
                projected("src/Main.elm", "y\n"),
            ])
        };
        let plain = build();
        let mut nerd = build();
        nerd.icons = Icons::new(IconStyle::Nerd);

        let plain_text = buffer_text(&render(&plain, 100, 20));
        let nerd_text = buffer_text(&render(&nerd, 100, 20));

        assert!(!plain_text.contains('\u{f07b}'), "no glyph by default");
        assert!(nerd_text.contains('\u{f07b}'), "folder glyph missing");
        assert!(nerd_text.contains('\u{e7a8}'), "rust glyph missing");
        assert!(nerd_text.contains('\u{e62c}'), "elm glyph missing");
        assert!(nerd_text.contains("main.rs"), "the label survives");
        assert!(nerd_text.contains("Main.elm"), "the label survives");
    }

    #[test]
    fn a_wide_terminal_shows_both_diff_sides_with_revision_labels() {
        let model = diff_model(vec![file_diff(
            "a.rs",
            Some("old line\n"),
            Some("new line\n"),
        )]);
        let text = buffer_text(&render(&model, 120, 20));

        assert!(text.contains("HEAD~1"), "old label missing: {text}");
        assert!(text.contains("HEAD"), "new label missing: {text}");
        assert!(text.contains("old line"), "old side missing: {text}");
        assert!(text.contains("new line"), "new side missing: {text}");
    }

    #[test]
    fn a_narrow_terminal_shows_only_the_focused_pane() {
        let model = model_with(vec![projected("a.rs", "pub fn a();\n")]);

        let tree = buffer_text(&render(&model, 60, 20));
        assert!(tree.contains("Files"));
        assert!(!tree.contains("pub fn a();"), "body must be hidden: {tree}");

        let mut body = model;
        body.focus = Pane::Body;
        let body = buffer_text(&render(&body, 60, 20));
        assert!(!body.contains("Files"), "tree must be hidden: {body}");
        assert!(body.contains("pub fn a();"));
    }

    #[test]
    fn a_tiny_terminal_shows_a_message_instead_of_a_frame() {
        let model = two_files();
        let text = buffer_text(&render(&model, 30, 5));
        assert!(text.contains("terminal too small"), "{text}");
    }

    #[test]
    fn the_help_overlay_lists_the_keys() {
        let mut model = two_files();
        model.overlay = Some(Overlay::Help);
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("help"), "{text}");
        assert!(text.contains("quit"), "{text}");
    }

    #[test]
    fn an_empty_result_renders_a_clear_empty_state() {
        let model = model_with(vec![projected("empty.rs", "")]);
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("No files"), "{text}");
        assert!(
            text.contains("No projected file content in this scope."),
            "{text}"
        );
    }

    #[test]
    fn a_diff_with_no_rows_renders_a_clear_empty_state() {
        let model = diff_model(Vec::new());
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("No changes"), "{text}");
        assert!(text.contains("Body-only edits are omitted."), "{text}");
    }

    #[test]
    fn a_long_line_is_clipped_and_never_overwrites_the_tree() {
        let long = "x".repeat(300);
        let model = model_with(vec![projected("a.rs", &format!("{long}\n"))]);
        let text = buffer_text(&render(&model, 100, 20));

        assert!(text.contains("Files"), "the tree must survive: {text}");
        assert!(!text.contains(&long), "the long line must be clipped");
        assert!(text.contains(&"x".repeat(40)), "the visible prefix remains");
    }

    #[test]
    fn a_long_diff_line_is_wrapped_inside_its_pane() {
        let long = "y".repeat(300);
        let model = diff_model(vec![file_diff(
            "a.rs",
            Some("x\n"),
            Some(&format!("{long}\n")),
        )]);
        let text = buffer_text(&render(&model, 120, 20));

        // Wrapping produces continuation rows, so the number gutter shows the
        // marker rather than repeating the source line number.
        assert!(text.contains('…'), "expected continuation markers: {text}");
        assert!(!text.contains(&long), "the long line must be wrapped");
    }

    #[test]
    fn the_show_pane_carries_syntax_colors() {
        if !crate::theme::colors_enabled() {
            return;
        }
        let model = model_with(vec![projected("a.rs", "pub struct User;\n")]);
        let buffer = render(&model, 100, 20);

        let colored = (0..buffer.area.height).any(|y| {
            (0..buffer.area.width).any(|x| {
                buffer
                    .cell((x, y))
                    .is_some_and(|cell| cell.fg != Color::Reset)
            })
        });
        assert!(colored, "expected a syntax foreground in the show pane");
    }

    #[test]
    fn a_changed_diff_row_gets_delta_backgrounds() {
        if !crate::theme::colors_enabled() {
            return;
        }
        let model = diff_model(vec![file_diff(
            "a.rs",
            Some("pub id: u32;\n"),
            Some("pub id: u64;\n"),
        )]);
        let buffer = render(&model, 120, 20);

        let theme = &model.theme;
        let delete_bg = theme.color(theme.palette.del_bg);
        let add_bg = theme.color(theme.palette.add_bg);
        let add_emph = theme.color(theme.palette.add_emph);
        let del_emph = theme.color(theme.palette.del_emph);
        let mut delete = false;
        let mut add = false;
        let mut emphasis = false;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    delete |= cell.bg == delete_bg;
                    add |= cell.bg == add_bg;
                    emphasis |= cell.bg == add_emph || cell.bg == del_emph;
                }
            }
        }
        assert!(delete, "expected a removed-line background");
        assert!(add, "expected an added-line background");
        assert!(emphasis, "expected intra-line emphasis");
    }

    #[test]
    fn the_canvas_is_opaque_in_every_flavor() {
        use crate::theme::{Capability, Flavor};

        for flavor in [Flavor::Dark, Flavor::Light] {
            // The help overlay is the largest: `Clear` resets its rect to the
            // terminal default before the popup repaints it, so it is the most
            // likely place for a leak.
            for overlay in [None, Some(Overlay::Help)] {
                let mut model = two_files();
                model.theme = Theme::new(flavor, Capability::TrueColor);
                model.overlay = overlay;
                let buffer = render(&model, 100, 20);
                for y in 0..buffer.area.height {
                    for x in 0..buffer.area.width {
                        let cell = buffer.cell((x, y)).expect("cell");
                        assert_ne!(
                            cell.bg,
                            Color::Reset,
                            "the terminal background leaked at ({x}, {y}) with {flavor:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_canvas_takes_the_light_palette_background() {
        use crate::theme::{Capability, Flavor};

        let mut model = two_files();
        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        let bg = light.color(light.palette.bg);
        model.theme = light;
        let buffer = render(&model, 100, 20);

        // The body pane interior must be the light background, not the dark one.
        let mut count = 0;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer.cell((x, y)).is_some_and(|cell| cell.bg == bg) {
                    count += 1;
                }
            }
        }
        assert!(count > 0, "expected the light background somewhere");
    }

    // Renders the file tree with Nerd Font icons. Run with
    // `--ignored --nocapture`.
    #[test]
    #[ignore = "prints a colored preview"]
    fn preview_nerd_icons() {
        let mut model = model_with(vec![
            projected("src/main.rs", "pub fn main() {}\n"),
            projected("src/Main.elm", "module Main exposing (..)\n"),
            projected("README.md", "# hi\n"),
        ]);
        model.icons = Icons::new(IconStyle::Nerd);
        let buffer = render(&model, 80, 12);
        print!("{}", ansi_preview(&buffer));
    }

    // Renders a diff and prints it with ANSI color, plus writes an HTML preview
    // to the system temporary directory. Run with `--ignored --nocapture`.
    #[test]
    #[ignore = "prints a colored preview"]
    fn preview_delta_colors() {
        let model = diff_model(vec![file_diff(
            "src/lib.rs",
            Some("pub struct User {\n    pub id: u32,\n    pub name: String,\n}\n"),
            Some("pub struct User {\n    pub id: u64,\n    pub name: String,\n}\n"),
        )]);
        let buffer = render(&model, 110, 14);
        print!("{}", ansi_preview(&buffer));

        let path = std::env::temp_dir().join("ownai-delta-preview.html");
        if std::fs::write(&path, html_preview(&buffer)).is_ok() {
            println!("\nHTML preview: {}", path.display());
        }
    }

    #[test]
    #[ignore = "prints a colored preview"]
    fn preview_light() {
        use crate::theme::{Capability, Flavor};

        let mut show = Model::new(
            "/repo".to_owned(),
            show_request(),
            "all".to_owned(),
            110,
            24,
            Theme::new(Flavor::Light, Capability::TrueColor),
            Icons::new(IconStyle::None),
        );
        show.install(
            Content::Show(
                vec![projected(
                    "src/lib.rs",
                    "// a comment\n#[derive(Debug)]\npub struct User<'a> {\n    pub id: u32,\n    pub name: &'a str,\n    pub tags: Vec<String>,\n}\n\nimpl<'a> User<'a> {\n    pub fn new(name: &'a str) -> Self {\n        let count = 42;\n        let msg = \"hello world\";\n        Self { id: count, name, tags: vec![msg.to_string()] }\n    }\n}\n",
                )]
                .into(),
            ),
            false,
        );
        let show = settle(show);
        print!("{}", ansi_preview(&render(&show, 110, 24)));

        let mut diff = Model::new(
            "/repo".to_owned(),
            diff_request(),
            "all".to_owned(),
            120,
            18,
            Theme::new(Flavor::Light, Capability::TrueColor),
            Icons::new(IconStyle::None),
        );
        diff.install(
            Content::Diff(
                vec![
                    file_diff("src/added.rs", None, Some("pub fn a();\n")),
                    file_diff(
                        "src/Mod.rs",
                        Some("pub fn old();\n"),
                        Some("pub fn new();\n"),
                    ),
                    file_diff("src/del.rs", Some("pub fn d();\n"), None),
                ]
                .into(),
            ),
            false,
        );
        let diff = settle(diff);
        let buffer = render(&diff, 120, 18);
        print!("{}", ansi_preview(&buffer));
        let path = std::env::temp_dir().join("ownai-light-preview.html");
        if std::fs::write(&path, html_preview(&buffer)).is_ok() {
            println!("\nHTML preview: {}", path.display());
        }
    }

    fn ansi_preview(buffer: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut current: Option<(Color, Color)> = None;
            for x in 0..buffer.area.width {
                let Some(cell) = buffer.cell((x, y)) else {
                    continue;
                };
                if current != Some((cell.fg, cell.bg)) {
                    out.push_str("\u{1b}[0m");
                    if let Color::Rgb(r, g, b) = cell.fg {
                        out.push_str(&format!("\u{1b}[38;2;{r};{g};{b}m"));
                    }
                    if let Color::Rgb(r, g, b) = cell.bg {
                        out.push_str(&format!("\u{1b}[48;2;{r};{g};{b}m"));
                    }
                    current = Some((cell.fg, cell.bg));
                }
                out.push_str(cell.symbol());
            }
            out.push_str("\u{1b}[0m\n");
        }
        out
    }

    fn html_preview(buffer: &ratatui::buffer::Buffer) -> String {
        let mut html = String::from(
            "<!doctype html><meta charset=utf-8><pre style=\"font:14px ui-monospace,Menlo,monospace;background:#1e1f1c;padding:12px\">",
        );
        for y in 0..buffer.area.height {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                let Some(cell) = buffer.cell((x, y)) else {
                    continue;
                };
                let symbol = match cell.symbol() {
                    "<" => "&lt;".to_owned(),
                    ">" => "&gt;".to_owned(),
                    "&" => "&amp;".to_owned(),
                    other => other.to_owned(),
                };
                let fg = match cell.fg {
                    Color::Rgb(r, g, b) => format!("color:#{r:02x}{g:02x}{b:02x}"),
                    _ => "color:#d8dee9".to_owned(),
                };
                let bg = match cell.bg {
                    Color::Rgb(r, g, b) => format!(";background:#{r:02x}{g:02x}{b:02x}"),
                    _ => String::new(),
                };
                line.push_str(&format!("<span style=\"{fg}{bg}\">{symbol}</span>"));
            }
            html.push_str(&line);
            html.push('\n');
        }
        html.push_str("</pre>");
        html
    }

    #[test]
    fn responsive_boundaries_pick_the_right_layout() {
        let model = model_with(vec![projected("a.rs", "pub fn a();\n")]);

        let wide = buffer_text(&render(&model, 80, 20));
        assert!(wide.contains("Files") && wide.contains("pub fn a();"));

        let narrow = buffer_text(&render(&model, 79, 20));
        assert!(narrow.contains("Files") && !narrow.contains("pub fn a();"));

        let edge = buffer_text(&render(&model, 40, 8));
        assert!(!edge.contains("terminal too small"));

        assert!(buffer_text(&render(&model, 39, 8)).contains("terminal too small"));
        assert!(buffer_text(&render(&model, 100, 7)).contains("terminal too small"));
    }

    #[test]
    fn horizontal_scroll_shifts_the_visible_columns() {
        let mut model = model_with(vec![projected("a.rs", "abcdef\n")]);
        model.focus = Pane::Body;
        assert!(buffer_text(&render(&model, 100, 20)).contains("abcdef"));

        let (scrolled, _) = update(Msg::Key(Key::Char('l')), &model);
        let text = buffer_text(&render(&scrolled, 100, 20));
        assert!(text.contains("bcdef"), "the tail must remain: {text}");
        assert!(
            !text.contains("abcdef"),
            "the first column must scroll away"
        );
    }

    #[test]
    fn non_utf8_path_components_render_escaped() {
        let path =
            SupportedPath::new(RepoPath::new(b"src/\xFF/lib.rs".as_slice()).expect("valid path"))
                .expect("supported path");
        let file = ProjectedFile::try_new(path, vec![item("x\n")]).expect("valid fixture");
        let model = model_with(vec![file]);

        let labels: Vec<String> = model.rows.iter().map(|row| row.label.clone()).collect();
        assert!(labels.iter().any(|label| label == "src"), "{labels:?}");
        assert!(
            labels.iter().any(|label| label.contains("\\xFF")),
            "{labels:?}"
        );
    }
}
