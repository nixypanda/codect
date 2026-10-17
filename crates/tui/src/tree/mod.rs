//! The file-tree plugin.
//!
//! It owns the navigator pane: the fold state, the derived visible rows, the
//! cursor, and the viewport. It never owns the projection's selected path;
//! moving the cursor onto a file asks the kernel to select it, and the kernel
//! writes the selection on [`crate::workspace::Workspace`].
//!
//! Tasks 4.3–4.9 extract the other surfaces. This plugin absorbs
//! `component/tree.rs` and the tree half of the old shell.

mod view;

use std::collections::BTreeSet;
use std::sync::Arc;

use base::RepoPath;
use engine::Engine;
use ratatui::Frame;

use crate::kernel::command::{Binding, CommandId, CommandSpec, KeyScope, KeySeq, Owner};
use crate::kernel::cx::Cx;
use crate::kernel::help::{HelpEntry, MOUSE, VIEW};
use crate::kernel::layout::{PaneGeometry, PaneSpec, Placement};
use crate::kernel::nav::{Nav, page_step};
use crate::kernel::plugin::{
    Event, FocusKey, HintTarget, Input, MouseInput, NavigatorResize, Never, Out, PaneId, Plugin,
    PluginId, Reply, Request,
};
use crate::plugin::{TREE_FOCUS, TREE_PANE};
use crate::util::input::{Key, MouseKind};

const MOUSE_SCROLL_STEP: i32 = 3;

/// A visible tree row: a directory or a file.
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    pub label: String,
    pub kind: RowKind,
}

/// The tree plugin's state.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub collapsed: BTreeSet<RepoPath>,
    pub rows: Arc<[TreeRow]>,
    pub cursor: usize,
    pub scroll: usize,
    pub viewport: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Msg {
    Move(i32),
    Page(i32),
    ToTop,
    ToBottom,
    Open,
    Close,
    Toggle,
    Click(usize),
    Scroll(i32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OutMsg {
    Selected(RepoPath),
}

impl Tree {
    fn sync(&mut self, visible: &[RepoPath]) {
        self.rows = Arc::from(build_rows(visible, &self.collapsed));
        self.collapsed
            .retain(|path| self.rows.iter().any(|row| row.kind.path() == path));
        if !self.rows.is_empty() {
            self.cursor = self.cursor.min(self.rows.len() - 1);
            self.scroll = self.scroll.min(self.rows.len() - 1);
        } else {
            self.cursor = 0;
            self.scroll = 0;
        }
    }

    fn reveal(&mut self, path: &RepoPath, height: usize) {
        if let Some(index) = self.row_of_file(path) {
            self.cursor = index;
            self.ensure_cursor_visible(height);
        }
    }

    pub(crate) fn row_of_file(&self, path: &RepoPath) -> Option<usize> {
        self.rows.iter().position(
            |row| matches!(&row.kind, RowKind::File { path: candidate } if candidate == path),
        )
    }

    fn row_of_directory(&self, path: &RepoPath) -> Option<usize> {
        self.rows.iter().position(|row| {
            matches!(&row.kind, RowKind::Directory { path: candidate, .. } if candidate == path)
        })
    }

    fn first_file_row(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row.kind, RowKind::File { .. }))
    }

    pub(crate) fn current_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.cursor)
    }

    fn update(&mut self, msg: Msg, visible: &[RepoPath], height: usize) -> Option<OutMsg> {
        let scrolled = matches!(&msg, Msg::Scroll(_));
        let out = match msg {
            Msg::Move(delta) => self.move_cursor(delta),
            Msg::Page(delta) => self.move_cursor(delta),
            Msg::ToTop => self.move_cursor_to(0),
            Msg::ToBottom => self.move_cursor_to(self.rows.len().saturating_sub(1)),
            Msg::Open => self.open(visible),
            Msg::Close => self.close(visible),
            Msg::Toggle => self.toggle(visible),
            Msg::Click(index) => self.click(index, visible),
            Msg::Scroll(delta) => {
                self.scroll_viewport(delta, height);
                None
            }
        };
        if !scrolled {
            self.ensure_cursor_visible(height);
        }
        out
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

    fn set_collapsed(&mut self, path: RepoPath, collapsed: bool, visible: &[RepoPath]) {
        if collapsed {
            self.collapsed.insert(path.clone());
        } else {
            self.collapsed.remove(&path);
        }
        self.rows = Arc::from(build_rows(visible, &self.collapsed));
        self.cursor = self.row_of_directory(&path).unwrap_or(0);
    }

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

    fn fit_viewport(&mut self, height: usize) {
        let shrunk = height < self.viewport;
        self.viewport = height;
        self.scroll = self.scroll.min(self.max_scroll(height));
        if shrunk {
            self.ensure_cursor_visible(height);
        }
    }

    fn max_scroll(&self, height: usize) -> usize {
        self.rows.len().saturating_sub(height)
    }

    fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll.saturating_add(height) {
            self.scroll = self.cursor + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll(height));
    }

    fn scroll_viewport(&mut self, delta: i32, height: usize) {
        let scroll = if delta < 0 {
            self.scroll.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.scroll.saturating_add(delta as usize)
        };
        self.scroll = scroll.min(self.max_scroll(height));
    }

    /// Apply a tree message at the current viewport height and translate a
    /// selection into a kernel request.
    fn apply(&mut self, msg: Msg, cx: &Cx) -> Vec<Out<Never>> {
        let height = viewport_height(cx);
        let visible = &cx.workspace.files.paths;
        match self.update(msg, visible, height) {
            Some(OutMsg::Selected(path)) => vec![Out::Kernel(Request::Select(Some(path)))],
            None => Vec::new(),
        }
    }

    fn on_files_changed(&mut self, cx: &Cx) -> Vec<Out<Never>> {
        let hint = self.cursor;
        self.sync(&cx.workspace.files.paths);
        match cx.workspace.selected.clone() {
            Some(path) => {
                let height = viewport_height(cx);
                self.reveal(&path, height);
                Vec::new()
            }
            None => match nearest_visible(self, hint) {
                Some(path) => vec![Out::Kernel(Request::Select(Some(path)))],
                None => {
                    if let Some(index) = self.first_file_row() {
                        self.cursor = index;
                    }
                    Vec::new()
                }
            },
        }
    }
}

/// The tree's command specs. The single source for the tree commands, shared by
/// the plugin and the legacy palette bridge.
pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        tree_spec("wider", "Widen file tree", "]", ']', 330),
        tree_spec("narrower", "Narrow file tree", "[", '[', 340),
        tree_spec("reset", "Reset file tree", "\\", '\\', 350),
    ]
}

fn tree_spec(
    name: &'static str,
    label: &'static str,
    hint: &'static str,
    character: char,
    rank: u16,
) -> CommandSpec {
    CommandSpec {
        id: CommandId {
            owner: Owner::Tree,
            name,
        },
        label,
        hint,
        bindings: vec![Binding {
            seq: KeySeq::Key(Key::Char(character)),
            scope: KeyScope::Anywhere,
        }],
        palette: true,
        group: None,
        rank,
    }
}

/// The inner height of the tree pane in the layout focused on the tree. This is
/// today's `tree_viewport_height`.
fn viewport_height(cx: &Cx) -> usize {
    cx.layout_if_focused(TREE_FOCUS)
        .slots
        .iter()
        .find(|slot| slot.pane == TREE_PANE)
        .map_or(0, |slot| slot.geometry.inner.height as usize)
}

fn tree_key(key: Key) -> Option<Msg> {
    match key {
        Key::Down | Key::Char('j') => Some(Msg::Move(1)),
        Key::Up | Key::Char('k') => Some(Msg::Move(-1)),
        Key::Right | Key::Char('l') => Some(Msg::Open),
        Key::Left | Key::Char('h') => Some(Msg::Close),
        Key::Enter => Some(Msg::Toggle),
        _ => None,
    }
}

fn tree_nav_msg(nav: Nav, step: u16) -> Msg {
    match nav {
        Nav::Top => Msg::ToTop,
        Nav::Bottom => Msg::ToBottom,
        Nav::PageUp => Msg::Page(-i32::from(step)),
        Nav::PageDown => Msg::Page(i32::from(step)),
    }
}

fn mouse_msg(state: &Tree, event: MouseInput) -> Option<Msg> {
    let inner = event.geometry.inner;
    match event.kind {
        MouseKind::Click => {
            let index = state.scroll + usize::from(event.row.saturating_sub(inner.y));
            (index < state.rows.len()).then_some(Msg::Click(index))
        }
        MouseKind::ScrollUp => Some(Msg::Scroll(-MOUSE_SCROLL_STEP)),
        MouseKind::ScrollDown => Some(Msg::Scroll(MOUSE_SCROLL_STEP)),
    }
}

fn nearest_visible(tree: &Tree, hint: usize) -> Option<RepoPath> {
    let file_rows: Vec<usize> = tree
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
    Some(tree.rows[chosen].kind.path().clone())
}

impl Plugin for Tree {
    const ID: PluginId = crate::plugin::TREE;
    type Msg = Never;
    type Cmd = Never;

    fn panes(&self, _cx: &Cx) -> Vec<PaneSpec> {
        vec![PaneSpec {
            id: TREE_PANE,
            placement: Placement::Navigator,
            focus: TREE_FOCUS,
        }]
    }

    fn has_focus(&self, focus: FocusKey) -> bool {
        focus == TREE_FOCUS
    }

    fn first_focus(&self) -> Option<FocusKey> {
        Some(TREE_FOCUS)
    }

    fn commands(&self, _cx: &Cx) -> Vec<CommandSpec> {
        commands()
    }

    fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
        vec![
            HelpEntry {
                section: VIEW,
                keys: "[ / ] / \\",
                text: "resize or reset the tree",
                rank: 60,
            },
            HelpEntry {
                section: MOUSE,
                keys: "click",
                text: "select a file or fold a directory",
                rank: 10,
            },
        ]
    }

    fn hints(&self, target: HintTarget, _cx: &Cx) -> Option<&'static str> {
        match target {
            HintTarget::Focus(focus) if focus == TREE_FOCUS => {
                Some("click open   Ctrl-P commands   Ctrl-F find   Tab content   ? help ")
            }
            HintTarget::Focus(_) | HintTarget::Overlay(_) => None,
        }
    }

    fn render(
        &self,
        pane: PaneId,
        geometry: PaneGeometry,
        focused: bool,
        cx: &Cx,
        frame: &mut Frame,
    ) {
        if pane == TREE_PANE {
            view::render(self, cx, frame, geometry.outer, focused, geometry.edge);
        }
    }

    fn input(&mut self, pane: PaneId, input: Input, cx: &Cx) -> Reply<Never> {
        if pane != TREE_PANE {
            return Reply::unhandled();
        }
        match input {
            Input::Key(key) => match tree_key(key) {
                Some(msg) => Reply::handled(self.apply(msg, cx)),
                None => Reply::unhandled(),
            },
            Input::Nav(nav) => {
                Reply::handled(self.apply(tree_nav_msg(nav, page_step(cx.size.height)), cx))
            }
            Input::Mouse(event) => Reply::handled(match mouse_msg(self, event) {
                Some(msg) => self.apply(msg, cx),
                None => Vec::new(),
            }),
        }
    }

    fn run(&mut self, command: CommandId, _cx: &Cx) -> Vec<Out<Never>> {
        if command.owner != Owner::Tree {
            return Vec::new();
        }
        let resize = match command.name {
            "wider" => NavigatorResize::Wider,
            "narrower" => NavigatorResize::Narrower,
            "reset" => NavigatorResize::Reset,
            _ => return Vec::new(),
        };
        vec![Out::Kernel(Request::ResizeNavigator(resize))]
    }

    fn on_event(&mut self, event: &Event, cx: &Cx) -> Vec<Out<Never>> {
        match event {
            Event::FilesChanged => self.on_files_changed(cx),
            Event::SelectionChanged => {
                if let Some(path) = cx.workspace.selected.as_ref() {
                    let height = viewport_height(cx);
                    self.reveal(path, height);
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn settle(&mut self, cx: &Cx) {
        let height = viewport_height(cx);
        self.fit_viewport(height);
    }

    fn perform(_engine: &Engine, cmd: Never) -> Never {
        match cmd {}
    }
}

pub(crate) fn build_rows(visible: &[RepoPath], collapsed: &BTreeSet<RepoPath>) -> Vec<TreeRow> {
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

    fn path(value: &str) -> RepoPath {
        RepoPath::new(value).unwrap()
    }

    #[test]
    fn build_rows_nests_directories_in_raw_path_order() {
        let visible = [path("a.rs"), path("src/lib.rs"), path("src/main.rs")];
        let rows = build_rows(&visible, &BTreeSet::new());
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["a.rs", "src", "lib.rs", "main.rs"]);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 0);
        assert_eq!(rows[2].depth, 1);
    }

    #[test]
    fn update_moves_the_cursor_and_reports_files() {
        let visible = [path("a.rs"), path("b.rs")];
        let mut tree = Tree::default();
        tree.sync(&visible);
        assert!(matches!(
            tree.update(Msg::Move(1), &visible, 10),
            Some(OutMsg::Selected(_))
        ));
        assert_eq!(tree.cursor, 1);
        assert!(tree.update(Msg::Scroll(3), &visible, 1).is_none());
    }

    #[test]
    fn collapsing_a_directory_hides_its_children() {
        let visible = [path("a.rs"), path("src/lib.rs"), path("src/main.rs")];
        let mut collapsed = BTreeSet::new();
        collapsed.insert(path("src"));
        let rows = build_rows(&visible, &collapsed);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["a.rs", "src"]);
        assert!(matches!(
            rows[1].kind,
            RowKind::Directory {
                expanded: false,
                ..
            }
        ));
    }
}
