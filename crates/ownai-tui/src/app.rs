//! The pure TEA core: `Model`, `Msg`, `Cmd`, and `update`.
//!
//! Nothing here performs I/O. `update` turns a message and the current model
//! into a replacement model plus a list of effects to run. Engine calls only
//! ever leave this module as a [`Cmd`], which the runtime interprets. The pure
//! `view` lives in [`crate::view`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ownai_core::{AreaSet, DiffRowKind, Language, ProjectedFile, ProjectionMode, RepoPath};
use ownai_engine::{EngineError, FileDiff, Selection, SelectionGroup};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::highlight::{self, StyledLine};
use crate::theme::Theme;
use crate::view::diff::layout_diff;
use crate::view::geom::{frame_chunks, gutter_width, pane_block, split_diff_columns};

/// At or above this width the tree and content render side by side.
pub(crate) const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

/// Below this width, or below [`MIN_HEIGHT`] rows, the terminal is too small.
pub(crate) const SINGLE_PANE_MIN_WIDTH: u16 = 40;
pub(crate) const MIN_HEIGHT: u16 = 8;

/// The file-tree pane width, as a percentage of the terminal, and its bounds.
const TREE_DEFAULT_PERCENT: u16 = 30;
const TREE_MIN_PERCENT: u16 = 15;
const TREE_MAX_PERCENT: u16 = 60;
const TREE_STEP: u16 = 5;

/// Which region currently has focus.
///
/// `Body` is the single projection pane of a `show`; `Diff` is both diff panes
/// treated as one focus unit, so `Tab` only ever toggles between the tree and
/// the content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Tree,
    Body,
    Diff,
}

/// A key the frontend understands, already translated from a terminal event.
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
    CtrlC,
}

/// A single-line editable field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextInput {
    pub text: String,
    /// A byte offset into `text`, always on a character boundary.
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

    /// The trimmed value to apply.
    pub fn value(&self) -> String {
        self.text.trim().to_owned()
    }
}

/// Which revision a revision prompt edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionField {
    Show,
    Base,
    Target,
}

/// A modal interaction that captures keys until it closes.
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
    },
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
                ..
            } => Self::Diff {
                base: base.clone(),
                target: target.clone(),
                mode,
                selection: selection.clone(),
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
                ..
            } => Self::Diff {
                base,
                target: target.clone(),
                mode: *mode,
                selection: selection.clone(),
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
                ..
            } => Self::Diff {
                base: base.clone(),
                target,
                mode: *mode,
                selection: selection.clone(),
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
                base, target, mode, ..
            } => Self::Diff {
                base: base.clone(),
                target: target.clone(),
                mode: *mode,
                selection,
            },
        }
    }

    /// The selection this request projects.
    pub fn selection(&self) -> &Selection {
        match self {
            Self::Show { selection, .. } | Self::Diff { selection, .. } => selection,
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

/// The loaded projection, either a single revision or a two-revision diff.
///
/// The payloads sit behind an `Arc` so cloning a `Model` is O(1) regardless of
/// how many files or bytes a projection holds.
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
    /// Paths the tree shows. A show hides files whose projection is empty; a
    /// diff already contains only paths whose projections differ.
    fn visible_paths(&self) -> Vec<RepoPath> {
        match self {
            Self::Show(files) => files
                .iter()
                .filter(|file| !file.canonical_text().is_empty())
                .map(|file| file.path().clone())
                .collect(),
            Self::Diff(diffs) => diffs.iter().map(|diff| diff.path.clone()).collect(),
        }
    }
}

/// Every input the core accepts.
#[derive(Debug)]
pub enum Msg {
    /// A key was pressed.
    Key(Key),
    /// The terminal changed size.
    Resize { width: u16, height: u16 },
    /// A projection effect finished. The request it ran for is echoed back.
    Loaded {
        request: LoadRequest,
        result: Result<Content, Box<EngineError>>,
    },
    /// The area configuration finished loading for the scope chooser.
    AreasLoaded(Result<AreaSet, Box<EngineError>>),
}

/// An effect the runtime must interpret. I/O is data, never a side effect of
/// `update`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    Load { request: LoadRequest },
    LoadAreas,
}

/// A row in the visible file tree.
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

/// The kind of a rendered diff row: either an aligned diff row or a synthetic
/// hunk header inserted where context was collapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowKind {
    Diff(DiffRowKind),
    Hunk,
}

/// How a file changed in a focused diff, for tree badges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// One visual row of a wrapped side-by-side diff.
///
/// A logical aligned row with a wrapped side expands into several `VisualRow`s;
/// only the first carries line numbers and the rest are continuation rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub kind: VisualRowKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_runs: StyledLine,
    pub new_runs: StyledLine,
    pub continuation: bool,
}

/// Syntax-highlighted lines for the selected diff, one side at a time.
#[derive(Clone, Default)]
pub struct DiffHighlight {
    pub old: Vec<StyledLine>,
    pub new: Vec<StyledLine>,
}

/// Highlighted lines for the selected file, keyed by path.
///
/// Highlighting is pure but not free, so it is computed once per file when the
/// selection first reaches it and reused while the user stays on that file.
#[derive(Clone, Default)]
struct HighlightCache {
    show: BTreeMap<RepoPath, Arc<Vec<StyledLine>>>,
    diff: BTreeMap<RepoPath, Arc<DiffHighlight>>,
}

/// The inputs a cached diff layout depends on.
#[derive(Clone, PartialEq, Eq)]
struct DerivedKey {
    generation: u64,
    width: u16,
    height: u16,
    selected: Option<RepoPath>,
    tree_percent: u16,
}

/// Expensive rendering state derived from the model, cached so a frame or a
/// scroll key does not recompute it. Held behind an `Arc`, so cloning a `Model`
/// never clones the layout.
#[derive(Clone, Default)]
struct Derived {
    key: Option<DerivedKey>,
    diff_rows: Vec<VisualRow>,
}

/// The entire UI state. It is replaced wholesale by `update`, never edited in
/// place by anything else.
///
/// Every large collection sits behind an `Arc`, so a clone is O(1); the only
/// deep data is small (fold state, highlight-cache entries).
#[derive(Clone)]
pub struct Model {
    pub root: String,
    /// The request that produced `content`; retained so `m` can re-project.
    pub request: LoadRequest,
    pub mode: ProjectionMode,
    pub scope_label: String,
    pub content: Content,
    /// Paths the tree shows, in raw path-byte order.
    pub visible: Arc<[RepoPath]>,
    /// Directories the user folded. Everything is expanded by default.
    pub collapsed: BTreeSet<RepoPath>,
    pub rows: Arc<[TreeRow]>,
    pub cursor: usize,
    pub selected: Option<RepoPath>,
    pub focus: Pane,
    pub body_scroll: u16,
    pub body_hscroll: u16,
    /// The file-tree pane width as a percentage of the terminal.
    pub tree_percent: u16,
    /// The active modal overlay, if any. It captures keys until it closes.
    pub overlay: Option<Overlay>,
    pub diagnostic: Option<String>,
    /// Lazily computed syntax highlighting for the selected file.
    highlights: HighlightCache,
    /// Bumped whenever `content` is replaced, invalidating [`Derived`].
    generation: u64,
    derived: Arc<Derived>,
    pub width: u16,
    pub height: u16,
    /// The resolved design tokens; view code never names a raw color.
    pub theme: Theme,
    pub quit: bool,
}

impl Model {
    /// A model with no projection loaded yet.
    pub fn new(
        root: String,
        request: LoadRequest,
        scope_label: String,
        width: u16,
        height: u16,
        theme: Theme,
    ) -> Self {
        let mode = request.mode();
        Self {
            root,
            request,
            mode,
            scope_label,
            content: Content::Show(Arc::from(Vec::new())),
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
            diagnostic: None,
            highlights: HighlightCache::default(),
            generation: 0,
            derived: Arc::new(Derived::default()),
            width,
            height,
            theme,
            quit: false,
        }
    }

    /// The projection currently shown by a `show` body, if any.
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

    /// The diff currently shown by the old and new panes, if any.
    pub fn active_diff(&self) -> Option<&FileDiff> {
        let path = self.selected.as_ref()?;
        match &self.content {
            Content::Diff(diffs) => diffs.iter().find(|diff| &diff.path == path),
            Content::Show(_) => None,
        }
    }

    /// The syntax-highlighted lines of the selected `show` file, if computed.
    pub(crate) fn active_show_lines(&self) -> Option<&[StyledLine]> {
        let path = self.selected.as_ref()?;
        self.highlights.show.get(path).map(|lines| lines.as_slice())
    }

    /// The syntax-highlighted lines of the selected diff, if computed.
    pub(crate) fn active_diff_highlight(&self) -> Option<&DiffHighlight> {
        let path = self.selected.as_ref()?;
        self.highlights.diff.get(path).map(|lines| lines.as_ref())
    }

    /// The wrapped visual rows of the active diff.
    pub(crate) fn diff_rows(&self) -> &[VisualRow] {
        self.derived.diff_rows.as_slice()
    }

    /// The change kind of a path in the active diff, for tree badges.
    pub(crate) fn change_kind(&self, path: &RepoPath) -> Option<ChangeKind> {
        let Content::Diff(diffs) = &self.content else {
            return None;
        };
        let diff = diffs.iter().find(|diff| &diff.path == path)?;
        Some(match (&diff.old, &diff.new) {
            (None, Some(_)) => ChangeKind::Added,
            (Some(_), None) => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        })
    }

    /// Computes and caches highlighting for the selected file when missing.
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
                let lines = highlight::highlight(file.canonical_text(), file.language(), &self.theme);
                self.highlights.show.insert(path, Arc::new(lines));
            }
            Content::Diff(diffs) => {
                if self.highlights.diff.contains_key(&path) {
                    return;
                }
                let Some(diff) = diffs.iter().find(|diff| diff.path == path) else {
                    return;
                };
                let language = diff
                    .old
                    .as_ref()
                    .or(diff.new.as_ref())
                    .map_or(Language::Rust, ProjectedFile::language);
                let old = diff.old.as_ref().map_or_else(Vec::new, |file| {
                    highlight::highlight(file.canonical_text(), language, &self.theme)
                });
                let new = diff.new.as_ref().map_or_else(Vec::new, |file| {
                    highlight::highlight(file.canonical_text(), language, &self.theme)
                });
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

    /// Recomputes the cached diff layout when its inputs changed.
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

    /// The wrapped visual rows of the active diff.
    fn compute_diff_rows(&self) -> Vec<VisualRow> {
        let Some(diff) = self.active_diff() else {
            return Vec::new();
        };
        let (content, _) = frame_chunks(self.width, self.height);
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
            let (old, new) = {
                let (_, old, new) = split_diff_columns(content, self.tree_percent);
                (old, new)
            };
            let old_inner = pane_block("", false, &self.theme).inner(old);
            let new_inner = pane_block("", false, &self.theme).inner(new);
            (
                (old_inner.width as usize).saturating_sub(gutter),
                (new_inner.width as usize).saturating_sub(gutter),
            )
        } else {
            let inner = pane_block("", false, &self.theme).inner(content);
            let width = (inner.width as usize).saturating_sub(gutter);
            (width, width)
        }
    }

    /// Installs a freshly loaded projection, preserving the selected path when
    /// it is still visible and choosing the nearest visible file otherwise.
    fn install(&mut self, content: Content, keep_selection: bool) {
        let previous = self.selected.clone();
        let hint = self.cursor;

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
        self.ensure_highlight();
        self.refresh_derived();
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

/// The panes `Tab` cycles through, for the current content.
fn focus_order(model: &Model) -> &'static [Pane] {
    match model.content {
        Content::Show(_) => &[Pane::Tree, Pane::Body],
        Content::Diff(_) => &[Pane::Tree, Pane::Diff],
    }
}

/// The pure update function. It performs no I/O and reads no external state.
pub fn update(msg: Msg, model: &Model) -> (Model, Vec<Cmd>) {
    let mut next = model.clone();
    let mut cmds = Vec::new();

    match msg {
        Msg::Key(key) => handle_key(key, &mut next, &mut cmds),
        Msg::Resize { width, height } => {
            next.width = width;
            next.height = height;
            next.refresh_derived();
            clamp_view(&mut next);
        }
        Msg::Loaded { request, result } => match result {
            Ok(content) => {
                next.mode = request.mode();
                next.scope_label = request.scope_label();
                next.request = request;
                next.install(content, true);
                next.diagnostic = None;
            }
            // A failed replacement leaves the previous model untouched.
            Err(error) => next.diagnostic = Some(error.to_string()),
        },
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

    next.refresh_derived();
    (next, cmds)
}

fn handle_key(key: Key, model: &mut Model, cmds: &mut Vec<Cmd>) {
    if key == Key::Char('q') || key == Key::CtrlC {
        model.quit = true;
        return;
    }
    if model.overlay.is_some() {
        overlay_key(key, model, cmds);
        return;
    }
    match key {
        Key::Char('?') => model.overlay = Some(Overlay::Help),
        Key::Esc => model.diagnostic = None,
        Key::Char('r') if matches!(model.content, Content::Show(_)) => {
            model.overlay = Some(Overlay::Revision {
                field: RevisionField::Show,
                input: TextInput::new(current_revision(model)),
            });
        }
        Key::Char('b') if matches!(model.content, Content::Diff(_)) => {
            model.overlay = Some(Overlay::Revision {
                field: RevisionField::Base,
                input: TextInput::new(current_base(model)),
            });
        }
        Key::Char('t') if matches!(model.content, Content::Diff(_)) => {
            model.overlay = Some(Overlay::Revision {
                field: RevisionField::Target,
                input: TextInput::new(current_target(model)),
            });
        }
        Key::Char('[') => {
            model.tree_percent = model
                .tree_percent
                .saturating_sub(TREE_STEP)
                .max(TREE_MIN_PERCENT);
        }
        Key::Char(']') => {
            model.tree_percent = (model.tree_percent + TREE_STEP).min(TREE_MAX_PERCENT);
        }
        Key::Char('\\') => model.tree_percent = TREE_DEFAULT_PERCENT,
        Key::Char('s') => {
            model.overlay = Some(Overlay::Scope(ScopeChooser::loading()));
            cmds.push(Cmd::LoadAreas);
        }
        Key::Char('m') => {
            let modes = available_modes();
            let cursor = modes
                .iter()
                .position(|mode| *mode == model.mode)
                .unwrap_or(0);
            model.overlay = Some(Overlay::Mode { cursor });
        }
        Key::Tab | Key::BackTab => cycle_focus(model, key == Key::Tab),
        _ => match model.focus {
            Pane::Tree => tree_key(key, model),
            Pane::Body | Pane::Diff => body_key(key, model),
        },
    }
}

/// Routes a key to the active overlay. The overlay is taken and reinserted so
/// its owned editor state can be mutated.
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
                                let label = path.to_string();
                                match Selection::new(vec![SelectionGroup::Path { label, path }]) {
                                    Ok(selection) => {
                                        apply_selection(model, cmds, selection);
                                        reopen = false;
                                    }
                                    Err(error) => {
                                        chooser.error = Some(error.to_string());
                                        chooser.input = Some(input);
                                    }
                                }
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
                        if let Some(area) = chooser
                            .areas
                            .as_ref()
                            .and_then(|areas| areas.names().nth(cursor - 1).map(str::to_owned))
                        {
                            let paths = chooser
                                .areas
                                .as_ref()
                                .and_then(|areas| areas.get(&area))
                                .map(|area| area.paths.clone())
                                .unwrap_or_default();
                            match Selection::new(vec![SelectionGroup::Area { name: area, paths }]) {
                                Ok(selection) => {
                                    apply_selection(model, cmds, selection);
                                    reopen = false;
                                }
                                Err(error) => chooser.error = Some(error.to_string()),
                            }
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
    }
}

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

/// Closes the active overlay and requests a reload with a new scope.
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
                    model.ensure_highlight();
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
    model.ensure_highlight();
}

fn directory_at_cursor(model: &Model) -> Option<(RepoPath, bool)> {
    match &model.current_row()?.kind {
        RowKind::Directory { path, expanded } => Some((path.clone(), *expanded)),
        RowKind::File { .. } => None,
    }
}

/// Folds or unfolds `path`, then keeps the cursor on that directory row.
fn set_collapsed(model: &mut Model, path: RepoPath, collapsed: bool) {
    if collapsed {
        model.collapsed.insert(path.clone());
    } else {
        model.collapsed.remove(&path);
    }
    model.rows = Arc::from(build_rows(&model.visible, &model.collapsed));
    model.cursor = model.row_of_directory(&path).unwrap_or(0);
}

/// Moves the cursor to the directory row that contains `path`, if one is
/// visible.
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

/// Derives the visible tree rows from the visible files.
///
/// Files arrive in raw path-byte order, so a directory's descendants form a
/// contiguous block and each directory row is emitted exactly once, at the
/// position of its first descendant. A collapsed directory emits only its own
/// row and skips its entire subtree.
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
    use crate::view::geom::{centered, window_offset};
    use crate::view::text::clip_line;
    use crate::view::view;
    use ownai_core::{Area, ItemKind, Language, ProjectedItem, SourceSpan};
    use ownai_engine::SelectionError;
    use ratatui::style::Color;

    fn item(text: &str) -> ProjectedItem {
        ProjectedItem {
            stable_key: "item".to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: "item".to_owned(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: 0,
                start_line: 0,
                start_column: 0,
                end_line: 0,
                end_column: 0,
            },
            canonical_text: text.to_owned(),
        }
    }

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path = RepoPath::new(path).expect("valid path");
        if text.is_empty() {
            return ProjectedFile::new(path, Language::Rust, Vec::new());
        }
        ProjectedFile::new(path, Language::Rust, vec![item(text)])
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
        }
    }

    fn file_diff(path: &str, old: Option<&str>, new: Option<&str>) -> FileDiff {
        FileDiff {
            path: RepoPath::new(path).expect("valid path"),
            old: old.map(|text| projected(path, text)),
            new: new.map(|text| projected(path, text)),
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
        );
        model.install(Content::Show(files.into()), false);
        model
    }

    fn diff_model(diffs: Vec<FileDiff>) -> Model {
        let mut model = Model::new(
            "/repo".to_owned(),
            diff_request(),
            "all".to_owned(),
            100,
            30,
            Theme::dark(),
        );
        model.install(Content::Diff(diffs.into()), false);
        model
    }

    fn two_files() -> Model {
        model_with(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")])
    }

    #[test]
    fn mode_picker_defers_the_mode_change_until_selection() {
        let model = two_files();
        let (opened, cmds) = update(Msg::Key(Key::Char('m')), &model);
        assert!(cmds.is_empty());
        assert!(matches!(opened.overlay, Some(Overlay::Mode { .. })));

        // The cursor starts on the current mode (types, index 0); move to
        // signatures and confirm.
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
        let text = buffer_text(&render(&model, 120, 20));
        assert!(text.contains("types"), "{text}");
        assert!(text.contains("scope:"), "{text}");
        assert!(text.contains("help"), "{text}");
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
        let error = EngineError::Selection(SelectionError::EmptyGroup {
            label: "x".to_owned(),
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
            Area {
                name: "core".to_owned(),
                paths: vec![RepoPath::new("src/core").unwrap()],
            },
            Area {
                name: "web".to_owned(),
                paths: vec![RepoPath::new("src/web").unwrap()],
            },
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
        let expected = Selection::new(vec![SelectionGroup::Area {
            name: "core".to_owned(),
            paths: vec![RepoPath::new("src/core").unwrap()],
        }])
        .unwrap();
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
        let error = EngineError::Selection(SelectionError::EmptyGroup {
            label: "x".to_owned(),
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

        // all(0), core(1), web(2), path…(3)
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
        let expected = Selection::new(vec![SelectionGroup::Path {
            label: "src/lib.rs".to_owned(),
            path: RepoPath::new("src/lib.rs").unwrap(),
        }])
        .unwrap();
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

        // The long old line wraps into 3 segments, so that logical row is 3
        // visual rows; the second logical row adds one more.
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

    #[test]
    fn a_wide_terminal_shows_the_tree_and_the_projection() {
        let model = model_with(vec![projected("a.rs", "pub fn a();\n")]);
        let text = buffer_text(&render(&model, 100, 20));

        assert!(text.contains("Files"), "tree pane missing: {text}");
        assert!(text.contains("pub fn a();"), "body missing: {text}");
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
        assert!(text.contains("no projected files"), "{text}");
    }

    #[test]
    fn a_diff_with_no_rows_renders_a_clear_empty_state() {
        let model = diff_model(Vec::new());
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("no projected changes"), "{text}");
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

    /// Renders a diff and prints it with ANSI color, plus writes an HTML preview
    /// to the system temporary directory. Run with `--ignored --nocapture`.
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
    fn a_zero_sized_rectangle_does_not_panic() {
        let model = two_files();
        let text = buffer_text(&render(&model, 1, 1));
        assert!(text.contains("terminal too small") || !text.is_empty());
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
    fn centered_handles_a_zero_sized_area() {
        let rect = centered(Rect::new(0, 0, 0, 0), 40, 10);
        assert_eq!(rect.width, 0);
        assert_eq!(rect.height, 0);
    }

    #[test]
    fn non_utf8_path_components_render_escaped() {
        let path = RepoPath::new(b"src/\xFF/lib.rs".as_slice()).expect("valid path");
        let file = ProjectedFile::new(path, Language::Rust, vec![item("x\n")]);
        let model = model_with(vec![file]);

        let labels: Vec<String> = model.rows.iter().map(|row| row.label.clone()).collect();
        assert!(labels.iter().any(|label| label == "src"), "{labels:?}");
        assert!(
            labels.iter().any(|label| label.contains("\\xFF")),
            "{labels:?}"
        );
    }
}
