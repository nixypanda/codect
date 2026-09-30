//! The file-tree component.
//!
//! It owns the fold state, the derived visible rows, and the cursor. It does not
//! own the projection's selected path: moving the cursor onto a file emits
//! [`OutMsg::Selected`], and the parent writes the selection where the content
//! kind owns it.

use std::collections::BTreeSet;
use std::sync::Arc;

use base::RepoPath;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::UnicodeWidthStr;

use crate::content::{ChangeKind, Loaded};
use crate::layout::{Edge, window_offset};
use crate::text::truncate_ellipsis;
use crate::theme::Theme;
use crate::view::block::pane_block;
use crate::view::empty::render_empty;

use super::RenderCtx;

/// A row in the visible file tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowKind {
    Directory { path: RepoPath, expanded: bool },
    File { path: RepoPath },
}

impl RowKind {
    pub fn path(&self) -> &RepoPath {
        match self {
            RowKind::Directory { path, .. } | RowKind::File { path } => path,
        }
    }
}

/// One visible tree row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    pub kind: RowKind,
}

/// The tree component's state.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    /// Directories the user folded. Everything is expanded by default.
    pub collapsed: BTreeSet<RepoPath>,
    pub rows: Arc<[TreeRow]>,
    pub cursor: usize,
}

/// A message the tree understands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Msg {
    /// Move the cursor by a signed number of rows.
    Move(i32),
    /// Page the cursor by a signed number of rows.
    Page(i32),
    ToTop,
    ToBottom,
    /// Expand the directory under the cursor, or step into it if it is already
    /// expanded.
    Open,
    /// Fold the directory under the cursor, or move to the containing directory.
    Close,
    /// Fold or unfold the directory under the cursor.
    Toggle,
    /// Select or fold the row at an absolute index, from a click.
    Click(usize),
}

/// What the tree asks its parent to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutMsg {
    /// The file under the cursor became the selection.
    Selected(RepoPath),
}

impl Tree {
    /// Replaces the rows for a new projection, dropping folds for directories
    /// that no longer exist and clamping the cursor.
    pub fn sync(&mut self, visible: &[RepoPath]) {
        self.rows = Arc::from(build_rows(visible, &self.collapsed));
        self.collapsed
            .retain(|path| self.rows.iter().any(|row| row.kind.path() == path));
        if !self.rows.is_empty() {
            self.cursor = self.cursor.min(self.rows.len() - 1);
        } else {
            self.cursor = 0;
        }
    }

    /// Puts the cursor on the row for a file, if it is visible.
    pub fn reveal(&mut self, path: &RepoPath) {
        if let Some(index) = self.row_of_file(path) {
            self.cursor = index;
        }
    }

    /// The index of the row for a file, if present.
    pub fn row_of_file(&self, path: &RepoPath) -> Option<usize> {
        self.rows.iter().position(
            |row| matches!(&row.kind, RowKind::File { path: candidate } if candidate == path),
        )
    }

    /// The index of the row for a directory, if present.
    pub fn row_of_directory(&self, path: &RepoPath) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(&row.kind, RowKind::Directory { path: candidate, .. } if candidate == path)
        })
    }

    /// The index of the first file row, if any.
    pub fn first_file_row(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row.kind, RowKind::File { .. }))
    }

    /// The row under the cursor, if any.
    pub fn current_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.cursor)
    }

    /// Applies a message. Folding rebuilds the rows from `visible`.
    pub fn update(&mut self, msg: Msg, visible: &[RepoPath]) -> Option<OutMsg> {
        match msg {
            Msg::Move(delta) => self.move_cursor(delta),
            Msg::Page(delta) => self.move_cursor(delta),
            Msg::ToTop => self.move_cursor_to(0),
            Msg::ToBottom => self.move_cursor_to(self.rows.len().saturating_sub(1)),
            Msg::Open => self.open(visible),
            Msg::Close => self.close(visible),
            Msg::Toggle => self.toggle(visible),
            Msg::Click(index) => self.click(index, visible),
        }
    }

    fn move_cursor(&mut self, delta: i32) -> Option<OutMsg> {
        if self.rows.is_empty() {
            return None;
        }
        let last = self.rows.len() - 1;
        let cursor = if delta < 0 {
            self.cursor.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            (self.cursor + delta as usize).min(last)
        };
        self.move_cursor_to(cursor)
    }

    fn move_cursor_to(&mut self, index: usize) -> Option<OutMsg> {
        if self.rows.is_empty() {
            return None;
        }
        self.cursor = index.min(self.rows.len() - 1);
        self.selected()
    }

    fn selected(&self) -> Option<OutMsg> {
        match self.current_row()?.kind.clone() {
            RowKind::File { path } => Some(OutMsg::Selected(path)),
            RowKind::Directory { .. } => None,
        }
    }

    fn open(&mut self, visible: &[RepoPath]) -> Option<OutMsg> {
        let (path, expanded) = self.directory_at_cursor()?;
        if expanded {
            self.move_cursor(1)
        } else {
            self.set_collapsed(path, false, visible);
            None
        }
    }

    fn close(&mut self, visible: &[RepoPath]) -> Option<OutMsg> {
        let row = self.current_row()?;
        let (path, expanded, is_dir) = match &row.kind {
            RowKind::Directory { path, expanded } => (path.clone(), *expanded, true),
            RowKind::File { path } => (path.clone(), false, false),
        };
        if is_dir && expanded {
            self.set_collapsed(path, true, visible);
            None
        } else {
            self.move_to_parent(&path);
            None
        }
    }

    fn toggle(&mut self, visible: &[RepoPath]) -> Option<OutMsg> {
        let row = self.current_row()?;
        match row.kind.clone() {
            RowKind::Directory { path, expanded } => {
                self.set_collapsed(path, expanded, visible);
                None
            }
            RowKind::File { path } => Some(OutMsg::Selected(path)),
        }
    }

    fn click(&mut self, index: usize, visible: &[RepoPath]) -> Option<OutMsg> {
        let kind = self.rows.get(index).map(|row| row.kind.clone())?;
        self.cursor = index;
        match kind {
            RowKind::Directory { path, expanded } => {
                self.set_collapsed(path, expanded, visible);
                None
            }
            RowKind::File { path } => Some(OutMsg::Selected(path)),
        }
    }

    fn directory_at_cursor(&self) -> Option<(RepoPath, bool)> {
        match &self.current_row()?.kind {
            RowKind::Directory { path, expanded } => Some((path.clone(), *expanded)),
            RowKind::File { .. } => None,
        }
    }

    /// Folds or unfolds `path`, then keeps the cursor on that directory row.
    fn set_collapsed(&mut self, path: RepoPath, collapsed: bool, visible: &[RepoPath]) {
        if collapsed {
            self.collapsed.insert(path.clone());
        } else {
            self.collapsed.remove(&path);
        }
        self.rows = Arc::from(build_rows(visible, &self.collapsed));
        self.cursor = self.row_of_directory(&path).unwrap_or(0);
    }

    /// Moves the cursor to the directory row that contains `path`, if visible.
    fn move_to_parent(&mut self, path: &RepoPath) {
        let Some(separator) = path.as_bytes().iter().rposition(|&byte| byte == b'/') else {
            return;
        };
        let parent =
            RepoPath::new(&path.as_bytes()[..separator]).expect("parent of a normalized path");
        if let Some(index) = self.row_of_directory(&parent) {
            self.cursor = index;
        }
    }
}

/// Derives the visible tree rows from the visible files.
///
/// Files arrive in raw path-byte order, so a directory's descendants form a
/// contiguous block and each directory row is emitted exactly once, at the
/// position of its first descendant. A collapsed directory emits only its own
/// row and skips its entire subtree.
pub fn build_rows(visible: &[RepoPath], collapsed: &BTreeSet<RepoPath>) -> Vec<TreeRow> {
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

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub(crate) fn render(
    state: &Tree,
    ctx: &RenderCtx,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let theme = ctx.theme;
    let block = pane_block(" Files ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.rows.is_empty() {
        let (title, detail) = if ctx.busy {
            ("Loading", "Projecting files…")
        } else {
            match ctx.loaded {
                Loaded::Show(_) => ("No files", "No projected file content in this scope."),
                Loaded::Diff(_) => ("No changes", "Body-only edits are omitted."),
            }
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    let height = inner.height as usize;
    let needs_scrollbar = state.rows.len() > height;
    let width = (inner.width as usize).saturating_sub(usize::from(needs_scrollbar));
    let offset = window_offset(state.cursor, state.rows.len(), height);
    let selected = ctx.loaded.selected();

    let mut lines = Vec::new();
    for (index, row) in state.rows.iter().enumerate().skip(offset).take(height) {
        lines.push(tree_line(
            state,
            ctx,
            row,
            index,
            index == state.cursor,
            focused,
            width,
            theme,
            selected,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if needs_scrollbar {
        let mut scrollbar = ScrollbarState::new(state.rows.len())
            .position(state.cursor)
            .viewport_content_length(height);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .style(theme.fg(theme.palette.border))
                .begin_symbol(None)
                .end_symbol(None),
            inner,
            &mut scrollbar,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn tree_line(
    state: &Tree,
    ctx: &RenderCtx,
    row: &TreeRow,
    index: usize,
    cursor: bool,
    focused: bool,
    width: usize,
    theme: &Theme,
    selected: Option<&RepoPath>,
) -> Line<'static> {
    let is_selected = matches!(&row.kind, RowKind::File { path } if Some(path) == selected);
    let highlighted = cursor && focused;
    let base = if is_selected || highlighted {
        let background = if highlighted {
            theme.palette.selection_bg
        } else {
            theme.palette.surface_alt
        };
        theme.fg_bg(theme.palette.selection_fg, background)
    } else {
        Style::default()
    };

    let mut spans: Vec<Span<'static>> = Vec::new();

    // The selection bar reserves its column on every row so labels stay aligned.
    if highlighted {
        spans.push(Span::styled(
            "▌".to_owned(),
            base.fg(theme.color(theme.palette.selection_bar)),
        ));
    } else if is_selected {
        spans.push(Span::styled(
            "▏".to_owned(),
            base.fg(theme.color(theme.palette.text_muted)),
        ));
    } else {
        spans.push(Span::styled(" ".to_owned(), base));
    }

    let rows = &state.rows;
    for level in 0..row.depth {
        let text = if level + 1 == row.depth {
            if is_last_child(rows, index, level) {
                "└─ "
            } else {
                "├─ "
            }
        } else if is_last_child(rows, index, level) {
            "   "
        } else {
            "│  "
        };
        spans.push(Span::styled(text.to_owned(), theme.fg(theme.palette.guide)));
    }

    let (marker, label_color) = match &row.kind {
        RowKind::Directory { expanded: true, .. } => ("▾ ", theme.palette.dir),
        RowKind::Directory {
            expanded: false, ..
        } => ("▸ ", theme.palette.dir),
        RowKind::File { .. } => ("", theme.palette.file),
    };
    if !marker.is_empty() {
        spans.push(Span::styled(
            marker.to_owned(),
            theme.fg(label_color).add_modifier(Modifier::BOLD),
        ));
    }

    let icon = match &row.kind {
        RowKind::Directory { .. } => ctx.icons.folder(),
        RowKind::File { path } => ctx.icons.file(path),
    };
    if !icon.is_empty() {
        let color = match &row.kind {
            RowKind::Directory { .. } => theme.palette.dir,
            RowKind::File { .. } => theme.palette.text_muted,
        };
        spans.push(Span::styled(format!("{icon} "), theme.fg(color)));
    }

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let badge = badge(ctx, row, theme);
    let badge_width = badge.as_ref().map_or(0, |(label, _)| label.len() + 1);
    let label_width = width.saturating_sub(used + badge_width);
    let label = truncate_ellipsis(&row.label, label_width);
    spans.push(Span::styled(label, base.fg(theme.color(label_color))));

    if let Some((label, color)) = badge {
        spans.push(Span::styled(
            format!(" {label}"),
            base.fg(theme.color(color)),
        ));
    }

    let used: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    if used < width {
        spans.push(Span::styled(" ".repeat(width - used), base));
    }
    // Keep the selection fill behind guides and icons as well as the label.
    for span in &mut spans {
        span.style = base.patch(span.style);
    }
    Line::from(spans)
}

fn badge(
    ctx: &RenderCtx,
    row: &TreeRow,
    theme: &Theme,
) -> Option<(&'static str, crate::theme::Rgb)> {
    let RowKind::File { path } = &row.kind else {
        return None;
    };
    match ctx.loaded.change_kind(path)? {
        ChangeKind::Added => Some(("A", theme.palette.badge_add)),
        ChangeKind::Modified => Some(("M", theme.palette.badge_mod)),
        ChangeKind::Deleted => Some(("D", theme.palette.badge_del)),
    }
}

fn is_last_child(rows: &[TreeRow], index: usize, level: usize) -> bool {
    for row in &rows[index + 1..] {
        if row.depth <= level {
            return row.depth < level;
        }
    }
    true
}
