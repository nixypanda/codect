//! The file-tree component.
//!
//! It owns the fold state, the derived visible rows, and the cursor. It does not
//! own the projection's selected path: moving the cursor onto a file emits
//! [`OutMsg::Selected`], and the parent writes the selection where the content
//! kind owns it.

use std::collections::BTreeSet;
use std::sync::Arc;

use base::RepoPath;

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
