//! The loaded projection: the discriminator that scopes focus, commits state,
//! prompts, and highlights to the content kind they belong to.
//!
//! A session loads exactly one kind — `show` or `diff` — and no key switches
//! between them. Making the kind the top-level sum is therefore safe, and it
//! removes the states the flat model allowed: a commits cursor with a `show`, a
//! horizontal scroll on a diff, a base-revision prompt on a `show`, and so on.

use std::sync::Arc;

use base::{FileDiff, ProjectedFile, ProjectionMode, RepoPath, Selection};

use crate::action::{Action, CommitsAction, GlobalAction, RangeAction, ShowAction};
use crate::cache::BoundedCache;
use crate::components::commit_picker::CommitPicker;
use crate::components::text_input::TextInput;
use crate::highlight::{self, StyledLine};
use crate::theme::Theme;

/// At or above this width the tree and content render side by side.
pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

/// The projection modes the frontend can select.
pub(crate) fn available_modes() -> [ProjectionMode; 2] {
    [ProjectionMode::Types, ProjectionMode::Signatures]
}

pub(crate) fn mode_label(mode: ProjectionMode) -> &'static str {
    match mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
    }
}

/// A projection scope: the selection and its short status-bar label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scope {
    pub selection: Selection,
    pub label: String,
}

impl Scope {
    pub fn new(selection: Selection) -> Self {
        let label = if selection.groups().is_empty() {
            "all".to_owned()
        } else {
            selection
                .groups()
                .iter()
                .map(base::SelectionGroup::label)
                .collect::<Vec<_>>()
                .join(", ")
        };
        Self { selection, label }
    }
}

/// Whether a projection is being loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Paging {
    Ready,
    Loading,
}

/// Which side of a diff a prompt edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffSide {
    Base,
    Target,
}

/// The base- or target-revision editor, which only exists on a diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffPrompt {
    pub side: DiffSide,
    pub input: TextInput,
}

/// Which projection a search match belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSide {
    Show,
    Old,
    New,
}

/// One occurrence of the search needle on a line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub side: SearchSide,
    /// One-based line number.
    pub line: usize,
    /// Byte range within the line.
    pub start: usize,
    pub end: usize,
}

/// A committed search: highlights persist after the input closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Search {
    pub needle: String,
    pub matches: Vec<SearchMatch>,
    pub cursor: usize,
}

/// Syntax-highlighted lines for the selected diff, one side at a time.
#[derive(Clone, Default)]
pub struct DiffHighlight {
    pub old: Vec<StyledLine>,
    pub new: Vec<StyledLine>,
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

/// Bounded, per-path syntax highlighting for a diff.
#[derive(Clone, Debug, Default)]
pub struct DiffHighlights {
    cache: BoundedCache<Arc<DiffHighlight>>,
}

impl DiffHighlights {
    pub fn highlight(&self, path: &RepoPath) -> Option<&DiffHighlight> {
        self.cache.get(path).map(Arc::as_ref)
    }

    pub fn contains(&self, path: &RepoPath) -> bool {
        self.cache.contains_key(path)
    }

    pub fn insert(&mut self, path: RepoPath, highlight: DiffHighlight) {
        self.cache.insert(path, Arc::new(highlight));
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

// ---------------------------------------------------------------------------
// Focus
// ---------------------------------------------------------------------------

/// The panes `Tab` cycles in a `show`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShowFocus {
    Tree,
    Body,
}

/// The panes `Tab` cycles in a diff range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeFocus {
    Tree,
    Diff,
}

/// The panes `Tab` cycles in a diff's commits view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitsFocus {
    Commits,
    Tree,
    Diff,
}

/// The body scroll offsets of a `show`: vertical and horizontal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShowBody {
    pub scroll: u16,
    pub hscroll: u16,
}

/// The body scroll offset of a diff. Diff panes always wrap, so there is no
/// horizontal offset to represent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffBody {
    pub scroll: u16,
}

// ---------------------------------------------------------------------------
// The loaded sum
// ---------------------------------------------------------------------------

/// The projection currently loaded.
#[derive(Clone, Debug)]
pub enum Loaded {
    Show(Show),
    Diff(Diff),
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

/// A loaded `diff` projection.
#[derive(Clone, Debug)]
pub struct Diff {
    pub base: String,
    pub target: String,
    pub mode: ProjectionMode,
    pub scope: Scope,
    pub visible: Arc<[RepoPath]>,
    pub selected: Option<RepoPath>,
    pub search: Option<Search>,
    pub paging: Paging,
    /// The base- or target-revision editor, which only exists on a diff.
    pub prompt: Option<DiffPrompt>,
    pub view: DiffViewState,
}

/// A diff's two views: the endpoint range, or the first-parent steps.
#[derive(Clone, Debug)]
pub enum DiffViewState {
    Range(Range),
    Commits(Commits),
}

/// The endpoint range: `base..target`, compared directly.
#[derive(Clone, Debug)]
pub struct Range {
    pub focus: RangeFocus,
    pub body: DiffBody,
    pub diffs: Arc<[FileDiff]>,
    pub highlight: DiffHighlights,
}

/// The commits view: a picker above the file tree, one step's diff shown.
#[derive(Clone, Debug)]
pub struct Commits {
    pub focus: CommitsFocus,
    pub picker: CommitPicker,
    pub body: DiffBody,
    pub diffs: Arc<[FileDiff]>,
    pub highlight: DiffHighlights,
}

impl Range {
    pub fn empty() -> Self {
        Self {
            focus: RangeFocus::Tree,
            body: DiffBody { scroll: 0 },
            diffs: Arc::from(Vec::new()),
            highlight: DiffHighlights::default(),
        }
    }
}

impl Commits {
    pub fn empty() -> Self {
        Self {
            focus: CommitsFocus::Tree,
            picker: CommitPicker::empty(),
            body: DiffBody { scroll: 0 },
            diffs: Arc::from(Vec::new()),
            highlight: DiffHighlights::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Loaded projection methods
// ---------------------------------------------------------------------------

impl Loaded {
    /// The tree paths the current projection shows.
    pub fn visible(&self) -> &[RepoPath] {
        match self {
            Self::Show(show) => &show.visible,
            Self::Diff(diff) => &diff.visible,
        }
    }

    pub fn selected(&self) -> Option<&RepoPath> {
        match self {
            Self::Show(show) => show.selected.as_ref(),
            Self::Diff(diff) => diff.selected.as_ref(),
        }
    }

    pub fn set_selected(&mut self, selected: Option<RepoPath>) {
        match self {
            Self::Show(show) => show.selected = selected,
            Self::Diff(diff) => diff.selected = selected,
        }
    }

    pub fn search(&self) -> Option<&Search> {
        match self {
            Self::Show(show) => show.search.as_ref(),
            Self::Diff(diff) => diff.search.as_ref(),
        }
    }

    pub fn mode(&self) -> ProjectionMode {
        match self {
            Self::Show(show) => show.mode,
            Self::Diff(diff) => diff.mode,
        }
    }

    pub fn scope(&self) -> &Scope {
        match self {
            Self::Show(show) => &show.scope,
            Self::Diff(diff) => &diff.scope,
        }
    }

    pub fn paging(&self) -> Paging {
        match self {
            Self::Show(show) => show.paging,
            Self::Diff(diff) => diff.paging,
        }
    }

    pub fn is_loading(&self) -> bool {
        self.paging() == Paging::Loading
    }

    /// The tree badge for a path, when a diff marks it.
    pub fn change_kind(&self, path: &RepoPath) -> Option<ChangeKind> {
        let Self::Diff(diff) = self else {
            return None;
        };
        let file = diff.diffs().iter().find(|file| file.path() == path)?;
        Some(match file {
            FileDiff::Added { .. } => ChangeKind::Added,
            FileDiff::Deleted { .. } => ChangeKind::Deleted,
            FileDiff::Modified { .. } => ChangeKind::Modified,
        })
    }

    /// The actions the command palette offers for this projection.
    pub fn palette_entries(&self) -> Vec<(Action, &'static str, &'static str)> {
        let mut entries = vec![
            (
                Action::Global(GlobalAction::Mode),
                "Switch Types / Signatures",
                "m",
            ),
            (Action::Global(GlobalAction::Scope), "Change scope", "s"),
        ];
        match self {
            Self::Show(_) => {
                entries.push((Action::Show(ShowAction::EditRevision), "Edit revision", "r"))
            }
            Self::Diff(diff) => match &diff.view {
                DiffViewState::Range(_) => {
                    entries.push((
                        Action::Range(RangeAction::EditBase),
                        "Edit base revision",
                        "b",
                    ));
                    entries.push((
                        Action::Range(RangeAction::EditTarget),
                        "Edit target revision",
                        "t",
                    ));
                    entries.push((
                        Action::Range(RangeAction::SwitchToCommits),
                        "Switch to commits view",
                        "",
                    ));
                }
                DiffViewState::Commits(_) => entries.push((
                    Action::Commits(CommitsAction::SwitchToRange),
                    "Switch to range view",
                    "",
                )),
            },
        }
        entries.extend([
            (Action::Global(GlobalAction::Finder), "Find file", "Ctrl-F"),
            (Action::Global(GlobalAction::Search), "Search in view", "/"),
            (Action::Global(GlobalAction::NextPane), "Switch pane", "Tab"),
            (
                Action::Global(GlobalAction::TreeWider),
                "Widen file tree",
                "]",
            ),
            (
                Action::Global(GlobalAction::TreeNarrower),
                "Narrow file tree",
                "[",
            ),
            (
                Action::Global(GlobalAction::TreeReset),
                "Reset file tree",
                "\\",
            ),
            (Action::Global(GlobalAction::Top), "Jump to top", "g"),
            (Action::Global(GlobalAction::Bottom), "Jump to bottom", "G"),
            (Action::Global(GlobalAction::Help), "Help", "?"),
            (Action::Global(GlobalAction::Quit), "Quit", "q"),
        ]);
        entries
    }
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
}

impl Diff {
    /// The diffs of the current view.
    pub fn diffs(&self) -> &[FileDiff] {
        match &self.view {
            DiffViewState::Range(range) => &range.diffs,
            DiffViewState::Commits(commits) => &commits.diffs,
        }
    }

    /// The diff currently shown, if a file is selected.
    pub fn active_diff(&self) -> Option<&FileDiff> {
        let path = self.selected.as_ref()?;
        self.diffs().iter().find(|diff| diff.path() == path)
    }

    pub fn focus_is_diff(&self) -> bool {
        match &self.view {
            DiffViewState::Range(range) => range.focus == RangeFocus::Diff,
            DiffViewState::Commits(commits) => commits.focus == CommitsFocus::Diff,
        }
    }

    pub fn body_scroll(&self) -> u16 {
        match &self.view {
            DiffViewState::Range(range) => range.body.scroll,
            DiffViewState::Commits(commits) => commits.body.scroll,
        }
    }

    pub fn set_body_scroll(&mut self, scroll: u16) {
        match &mut self.view {
            DiffViewState::Range(range) => range.body.scroll = scroll,
            DiffViewState::Commits(commits) => commits.body.scroll = scroll,
        }
    }

    /// The revisions the visible diff compares, as short labels.
    pub fn revisions(&self) -> (String, String) {
        if let DiffViewState::Commits(commits) = &self.view
            && let Some(step) = commits.picker.steps_slice().get(commits.picker.cursor())
        {
            let parent = step.parent_id.to_string();
            let commit = step.commit_id.to_string();
            return (
                parent[..parent.len().min(7)].to_owned(),
                commit[..commit.len().min(7)].to_owned(),
            );
        }
        (self.base.clone(), self.target.clone())
    }

    /// The search ranges on one line of one diff side.
    pub fn search_ranges(&self, side: SearchSide, line: usize) -> Vec<(usize, usize, bool)> {
        search_ranges(self.search.as_ref(), side, line)
    }
}

fn search_ranges(
    search: Option<&Search>,
    side: SearchSide,
    line: usize,
) -> Vec<(usize, usize, bool)> {
    let Some(search) = search else {
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

/// How a file changed in a focused diff, for tree badges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// Builds a show scope's visible paths from its files.
pub fn show_visible(files: &[ProjectedFile]) -> Vec<RepoPath> {
    files
        .iter()
        .filter(|file| !file.canonical_text().is_empty())
        .map(|file| file.path().clone())
        .collect()
}

/// Builds a diff scope's visible paths from its diffs.
pub fn diff_visible(diffs: &[FileDiff]) -> Vec<RepoPath> {
    diffs.iter().map(|diff| diff.path().clone()).collect()
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

impl Show {
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

    fn search_matches(&self, needle: &str) -> Vec<SearchMatch> {
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

impl Diff {
    fn search_matches(&self, needle: &str) -> Vec<SearchMatch> {
        let needle_lower = needle.to_lowercase();
        let mut matches = Vec::new();
        let Some(diff) = self.active_diff() else {
            return matches;
        };
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
        matches
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

    /// Computes and caches highlighting for the selected diff when missing.
    pub fn ensure_highlight(&mut self, theme: &Theme) {
        let Some(path) = self.selected.clone() else {
            return;
        };
        let Some(diff) = self
            .diffs()
            .iter()
            .find(|diff| *diff.path() == path)
            .cloned()
        else {
            return;
        };
        let language = match &diff {
            FileDiff::Added { new } => new.language(),
            FileDiff::Deleted { old } => old.language(),
            FileDiff::Modified { old, .. } => old.language(),
        };
        let highlight_side =
            |file: &ProjectedFile| highlight::highlight(file.canonical_text(), language, theme);
        match &mut self.view {
            DiffViewState::Range(range) => {
                if range.highlight.contains(&path) {
                    return;
                }
                let pair = diff_highlight(&diff, &highlight_side);
                range.highlight.insert(path, pair);
            }
            DiffViewState::Commits(commits) => {
                if commits.highlight.contains(&path) {
                    return;
                }
                let pair = diff_highlight(&diff, &highlight_side);
                commits.highlight.insert(path, pair);
            }
        }
    }

    /// The current view's highlight for the selected diff, if computed.
    pub fn active_highlight(&self) -> Option<&DiffHighlight> {
        let path = self.selected.as_ref()?;
        match &self.view {
            DiffViewState::Range(range) => range.highlight.highlight(path),
            DiffViewState::Commits(commits) => commits.highlight.highlight(path),
        }
    }
}

fn diff_highlight(
    diff: &FileDiff,
    highlight_side: &impl Fn(&ProjectedFile) -> Vec<StyledLine>,
) -> DiffHighlight {
    match diff {
        FileDiff::Added { new } => DiffHighlight {
            old: Vec::new(),
            new: highlight_side(new),
        },
        FileDiff::Deleted { old } => DiffHighlight {
            old: highlight_side(old),
            new: Vec::new(),
        },
        FileDiff::Modified { old, new } => DiffHighlight {
            old: highlight_side(old),
            new: highlight_side(new),
        },
    }
}
