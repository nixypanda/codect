//! The pure TEA core: `Model`, `Msg`, `Cmd`, `update`, and `view`.
//!
//! Nothing here performs I/O. `update` turns a message and the current model
//! into a replacement model plus a list of effects to run; `view` renders a
//! model into a frame. Engine calls only ever leave this module as a [`Cmd`],
//! which the runtime interprets.

use std::collections::BTreeSet;

use ownai_core::{ProjectedFile, ProjectionMode, RepoPath};
use ownai_engine::{EngineError, Selection};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// At or above this width the tree and projection render side by side.
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 80;

/// Below this width, or below [`MIN_HEIGHT`] rows, the terminal is too small.
const SINGLE_PANE_MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 8;

/// Columns a tab expands to, so display width stays deterministic.
const TAB_WIDTH: usize = 4;

/// Which pane currently has focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Pane {
    Tree,
    Body,
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
    CtrlC,
}

/// Every input the core accepts.
#[derive(Debug)]
pub enum Msg {
    /// A key was pressed.
    Key(Key),
    /// The terminal changed size.
    Resize { width: u16, height: u16 },
    /// A projection effect finished. `mode` is the mode it was run for.
    Loaded {
        mode: ProjectionMode,
        result: Result<Vec<ProjectedFile>, EngineError>,
    },
}

/// An effect the runtime must interpret. I/O is data, never a side effect of
/// `update`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    /// Project `revision` in `mode` with `selection`.
    LoadShow {
        revision: String,
        mode: ProjectionMode,
        selection: Selection,
    },
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

/// The entire UI state. It is replaced wholesale by `update`, never edited in
/// place by anything else.
#[derive(Clone)]
pub struct Model {
    pub root: String,
    pub revision: String,
    pub mode: ProjectionMode,
    pub scope_label: String,
    /// The selection used to produce `files`; retained so `m` can re-project.
    pub selection: Selection,
    /// Every supported file the engine returned, including empty projections.
    pub files: Vec<ProjectedFile>,
    /// Paths with a non-empty projection, in raw path-byte order; the tree is
    /// built from these and empty projections are hidden.
    pub visible: Vec<RepoPath>,
    /// Directories the user folded. Everything is expanded by default.
    pub collapsed: BTreeSet<RepoPath>,
    pub rows: Vec<TreeRow>,
    pub cursor: usize,
    pub selected: Option<RepoPath>,
    pub focus: Pane,
    pub body_scroll: u16,
    pub body_hscroll: u16,
    pub help: bool,
    pub diagnostic: Option<String>,
    pub width: u16,
    pub height: u16,
    pub quit: bool,
}

impl Model {
    /// A model with no projection loaded yet.
    pub fn new(
        root: String,
        revision: String,
        mode: ProjectionMode,
        scope_label: String,
        selection: Selection,
        width: u16,
        height: u16,
    ) -> Self {
        Self {
            root,
            revision,
            mode,
            scope_label,
            selection,
            files: Vec::new(),
            visible: Vec::new(),
            collapsed: BTreeSet::new(),
            rows: Vec::new(),
            cursor: 0,
            selected: None,
            focus: Pane::Tree,
            body_scroll: 0,
            body_hscroll: 0,
            help: false,
            diagnostic: None,
            width,
            height,
            quit: false,
        }
    }

    /// The projection currently shown in the body, if any.
    pub fn active_file(&self) -> Option<&ProjectedFile> {
        let path = self.selected.as_ref()?;
        self.files.iter().find(|file| file.path() == path)
    }

    pub fn active_text(&self) -> Option<&str> {
        self.active_file().map(ProjectedFile::canonical_text)
    }

    fn line_count(&self) -> usize {
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

    /// Installs a freshly loaded projection, preserving the selected path when
    /// it still has a non-empty projection and choosing the nearest visible
    /// file otherwise.
    fn install_files(&mut self, files: Vec<ProjectedFile>, keep_selection: bool) {
        let previous = self.selected.clone();
        let hint = self.cursor;

        self.files = files;
        self.visible = self
            .files
            .iter()
            .filter(|file| !file.canonical_text().is_empty())
            .map(|file| file.path().clone())
            .collect();
        self.rows = build_rows(&self.visible, &self.collapsed);
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
        self.body_scroll = 0;
        self.body_hscroll = 0;
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

/// The pure update function. It performs no I/O and reads no external state.
pub fn update(msg: Msg, model: &Model) -> (Model, Vec<Cmd>) {
    let mut next = model.clone();
    let mut cmds = Vec::new();

    match msg {
        Msg::Key(key) => handle_key(key, &mut next, &mut cmds),
        Msg::Resize { width, height } => {
            next.width = width;
            next.height = height;
            clamp_view(&mut next);
        }
        Msg::Loaded { mode, result } => match result {
            Ok(files) => {
                next.mode = mode;
                next.install_files(files, true);
                next.diagnostic = None;
            }
            // A failed replacement leaves the previous model untouched.
            Err(error) => next.diagnostic = Some(error.to_string()),
        },
    }

    (next, cmds)
}

fn handle_key(key: Key, model: &mut Model, cmds: &mut Vec<Cmd>) {
    if key == Key::Char('q') || key == Key::CtrlC {
        model.quit = true;
        return;
    }
    if key == Key::Char('?') {
        model.help = !model.help;
        return;
    }
    if key == Key::Esc {
        if model.help {
            model.help = false;
        } else {
            model.diagnostic = None;
        }
        return;
    }
    // The help overlay swallows the rest until it is dismissed.
    if model.help {
        return;
    }
    if key == Key::Tab || key == Key::BackTab {
        model.focus = match model.focus {
            Pane::Tree => Pane::Body,
            Pane::Body => Pane::Tree,
        };
        return;
    }
    if key == Key::Char('m') {
        let mode = match model.mode {
            ProjectionMode::Types => ProjectionMode::Signatures,
            ProjectionMode::Signatures => ProjectionMode::Types,
        };
        // The mode is only changed when the replacement arrives.
        cmds.push(Cmd::LoadShow {
            revision: model.revision.clone(),
            mode,
            selection: model.selection.clone(),
        });
        return;
    }

    match model.focus {
        Pane::Tree => tree_key(key, model),
        Pane::Body => body_key(key, model),
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
                }
            }
        }
        _ => {}
    }
}

fn body_key(key: Key, model: &mut Model) {
    match key {
        Key::Down | Key::Char('j') => {
            model.body_scroll = model.body_scroll.saturating_add(1).min(max_scroll(model));
        }
        Key::Up | Key::Char('k') => {
            model.body_scroll = model.body_scroll.saturating_sub(1);
        }
        Key::Right | Key::Char('l') => {
            model.body_hscroll = model.body_hscroll.saturating_add(1).min(max_hscroll(model));
        }
        Key::Left | Key::Char('h') => {
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
    let Some(row) = model.current_row() else {
        return;
    };
    if let RowKind::File { path } = &row.kind {
        let path = path.clone();
        if model.selected.as_ref() != Some(&path) {
            model.body_scroll = 0;
            model.body_hscroll = 0;
        }
        model.selected = Some(path);
    }
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
    model.rows = build_rows(&model.visible, &model.collapsed);
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
    let count = model.line_count();
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

/// Renders the whole model. Pure: it reads the model and writes to the frame.
pub fn view(model: &Model, frame: &mut Frame) {
    let area = frame.area();

    if area.width < SINGLE_PANE_MIN_WIDTH || area.height < MIN_HEIGHT {
        render_too_small(frame, area);
        if model.help {
            render_help(frame, area);
        }
        return;
    }

    let mut constraints = vec![Constraint::Min(1)];
    if model.diagnostic.is_some() {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    let chunks = Layout::vertical(constraints).split(area);

    let content = chunks[0];
    let status = chunks[chunks.len() - 1];
    let diagnostic = (model.diagnostic.is_some()).then(|| chunks[1]);

    if area.width >= SIDE_BY_SIDE_MIN_WIDTH {
        let columns = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(content);
        render_tree(model, frame, columns[0], model.focus == Pane::Tree);
        render_body(model, frame, columns[1], model.focus == Pane::Body);
    } else {
        match model.focus {
            Pane::Tree => render_tree(model, frame, content, true),
            Pane::Body => render_body(model, frame, content, true),
        }
    }

    render_status(model, frame, status);
    if let Some(area) = diagnostic {
        render_diagnostic(model, frame, area);
    }
    if model.help {
        render_help(frame, area);
    }
}

fn pane_block(title: &str, focused: bool) -> Block<'static> {
    let border = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(title.to_owned())
}

fn render_tree(model: &Model, frame: &mut Frame, area: Rect, focused: bool) {
    let block = pane_block(" Files ", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if model.rows.is_empty() {
        frame.render_widget(Paragraph::new("no projected files"), inner);
        return;
    }

    let height = inner.height as usize;
    let offset = window_offset(model.cursor, model.rows.len(), height);
    let mut lines = Vec::new();
    for (index, row) in model.rows.iter().enumerate().skip(offset).take(height) {
        let selected = index == model.cursor;
        let indent = "  ".repeat(row.depth);
        let marker = match &row.kind {
            RowKind::Directory { expanded: true, .. } => "▾ ",
            RowKind::Directory {
                expanded: false, ..
            } => "▸ ",
            RowKind::File { .. } => "  ",
        };
        let style = if selected && focused {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{indent}{marker}{}", row.label),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_body(model: &Model, frame: &mut Frame, area: Rect, focused: bool) {
    let title = model
        .selected
        .as_ref()
        .map_or_else(|| " Projection ".to_owned(), |path| format!(" {path} "));
    let block = pane_block(&title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(text) = model.active_text() else {
        frame.render_widget(Paragraph::new("no projected files"), inner);
        return;
    };

    let width = inner.width as usize;
    let height = inner.height as usize;
    let skip = model.body_scroll as usize;
    let hscroll = model.body_hscroll as usize;
    let mut lines = Vec::new();
    for line in text.lines().skip(skip).take(height) {
        lines.push(Line::from(clip_line(line, hscroll, width)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_status(model: &Model, frame: &mut Frame, area: Rect) {
    let selected = model
        .selected
        .as_ref()
        .map_or_else(|| "-".to_owned(), |path| path.to_string());
    let mode = match model.mode {
        ProjectionMode::Types => "types",
        ProjectionMode::Signatures => "signatures",
    };
    let text = format!(
        " {}  ·  {}  ·  {}  ·  scope: {}  ·  {} ",
        model.root, mode, model.revision, model.scope_label, selected
    );
    frame.render_widget(
        Paragraph::new(clip_line(&text, 0, area.width as usize)),
        area,
    );
}

fn render_diagnostic(model: &Model, frame: &mut Frame, area: Rect) {
    if let Some(text) = &model.diagnostic {
        let style = Style::default().fg(Color::Red);
        frame.render_widget(
            Paragraph::new(clip_line(text, 0, area.width as usize)).style(style),
            area,
        );
    }
}

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered(area, 48, 10);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from("  q / Ctrl-C    quit"),
        Line::from("  ↑ ↓ / k j     move or scroll"),
        Line::from("  ← → / h l     fold or scroll sideways"),
        Line::from("  Tab           switch pane"),
        Line::from("  Enter         open or fold"),
        Line::from("  m             switch mode"),
        Line::from("  Esc           close help or dismiss"),
    ];
    let block = Block::default().borders(Borders::ALL).title(" Help ");
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    frame.render_widget(Paragraph::new("terminal too small"), area);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn window_offset(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height || cursor < height {
        0
    } else {
        (cursor + 1).saturating_sub(height)
    }
}

/// A display-width slice of one line, expanded tabs included.
///
/// It never splits a code point: a wide character that straddles the cut is
/// dropped whole. Combining marks are kept with the base character they follow.
fn clip_line(line: &str, skip: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let expanded = if line.contains('\t') {
        line.replace('\t', &" ".repeat(TAB_WIDTH))
    } else {
        line.to_owned()
    };

    let mut out = String::new();
    let mut column = 0usize;
    let mut taken = 0usize;
    for character in expanded.chars() {
        let cells = UnicodeWidthChar::width(character).unwrap_or(0);
        if cells == 0 {
            if column >= skip {
                out.push(character);
            }
            continue;
        }
        if column + cells <= skip {
            column += cells;
            continue;
        }
        if column < skip {
            // The character straddles the cut; drop it rather than split it.
            column += cells;
            continue;
        }
        if taken + cells > width {
            break;
        }
        out.push(character);
        taken += cells;
        column += cells;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ownai_core::{ItemKind, Language, ProjectedItem, SourceSpan};
    use ownai_engine::SelectionError;

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

    fn model_with(files: Vec<ProjectedFile>) -> Model {
        let mut model = Model::new(
            "/repo".to_owned(),
            "HEAD".to_owned(),
            ProjectionMode::Types,
            "all".to_owned(),
            Selection::all(),
            100,
            30,
        );
        model.install_files(files, false);
        model
    }

    fn two_files() -> Model {
        model_with(vec![projected("a.rs", "a\n"), projected("b.rs", "b\n")])
    }

    #[test]
    fn mode_key_emits_a_cmd_and_defers_the_mode_change() {
        let model = two_files();
        let (next, cmds) = update(Msg::Key(Key::Char('m')), &model);

        assert_eq!(model.mode, ProjectionMode::Types);
        assert_eq!(next.mode, ProjectionMode::Types, "mode changes on Loaded");
        assert_eq!(
            cmds,
            vec![Cmd::LoadShow {
                revision: "HEAD".to_owned(),
                mode: ProjectionMode::Signatures,
                selection: Selection::all(),
            }]
        );
    }

    #[test]
    fn a_loaded_result_installs_files_and_preserves_the_selection() {
        let model = two_files();
        let (next, _) = update(
            Msg::Loaded {
                mode: ProjectionMode::Signatures,
                result: Ok(vec![projected("a.rs", "A\n"), projected("b.rs", "B\n")]),
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
        let model = model_with(vec![
            projected("a.rs", "a\n"),
            projected("b.rs", "b\n"),
            projected("c.rs", "c\n"),
        ]);
        let mut model = model;
        model.cursor = model.row_of_file(&RepoPath::new("b.rs").unwrap()).unwrap();
        model.selected = Some(RepoPath::new("b.rs").unwrap());

        let (next, _) = update(
            Msg::Loaded {
                mode: ProjectionMode::Types,
                result: Ok(vec![projected("a.rs", "a\n"), projected("c.rs", "c\n")]),
            },
            &model,
        );

        // `b.rs` is gone; the nearest surviving file is chosen, not a stale path.
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
                mode: ProjectionMode::Signatures,
                result: Err(error),
            },
            &model,
        );

        assert_eq!(next.mode, ProjectionMode::Types, "the mode must not change");
        assert_eq!(next.files.len(), 2, "the previous projection must survive");
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
    fn help_toggles_and_swallows_navigation() {
        let model = two_files();
        let (help, _) = update(Msg::Key(Key::Char('?')), &model);
        assert!(help.help);

        let (still, _) = update(Msg::Key(Key::Char('j')), &help);
        assert_eq!(still.cursor, help.cursor);

        let (closed, _) = update(Msg::Key(Key::Esc), &help);
        assert!(!closed.help);
    }

    #[test]
    fn clip_line_slices_by_display_width() {
        assert_eq!(clip_line("abcdef", 2, 3), "cde");
        assert_eq!(clip_line("abcdef", 0, 0), "");
        assert_eq!(clip_line("abcdef", 10, 3), "");
        assert_eq!(clip_line("a\tb", 0, 10), "a    b");
        // Wide characters are never split.
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
        model.help = true;
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("Help"), "{text}");
        assert!(text.contains("quit"), "{text}");
    }

    #[test]
    fn an_empty_result_renders_a_clear_empty_state() {
        let model = model_with(vec![projected("empty.rs", "")]);
        let text = buffer_text(&render(&model, 100, 20));
        assert!(text.contains("no projected files"), "{text}");
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
    fn a_zero_sized_rectangle_does_not_panic() {
        let model = two_files();
        let text = buffer_text(&render(&model, 1, 1));
        assert!(text.contains("terminal too small") || !text.is_empty());
    }

    #[test]
    fn responsive_boundaries_pick_the_right_layout() {
        let model = model_with(vec![projected("a.rs", "pub fn a();\n")]);

        // Exactly 80 columns is side by side.
        let wide = buffer_text(&render(&model, 80, 20));
        assert!(wide.contains("Files") && wide.contains("pub fn a();"));

        // 79 columns is one pane, and the tree has focus.
        let narrow = buffer_text(&render(&model, 79, 20));
        assert!(narrow.contains("Files") && !narrow.contains("pub fn a();"));

        // Exactly 40 columns is still a real single pane.
        let edge = buffer_text(&render(&model, 40, 8));
        assert!(!edge.contains("terminal too small"));

        // 39 columns and 7 rows are too small.
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
