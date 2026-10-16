//! The `commits` view's picker: a scrollable list of first-parent steps.
//!
//! It owns the cursor, the scroll offset, and the navigation target (the step
//! whose projection is being loaded while the displayed diff still belongs to
//! the previously loaded step). The list and cursor cannot outlive each other:
//! an empty list is a distinct variant with no cursor to dangle.

use std::sync::Arc;

use engine::CommitStep;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::render::block::pane_block;
use crate::render::empty::render_empty;
use crate::render::layout::{Edge, commit_offset};
use crate::render::text::truncate_ellipsis;

use super::RenderCtx;

#[derive(Clone, Debug)]
pub enum CommitList {
    Empty,
    Steps {
        steps: Arc<[CommitStep]>,
        cursor: usize,
        scroll: usize,
        target: Option<usize>,
    },
}

#[derive(Clone, Debug)]
pub struct CommitPicker {
    pub list: CommitList,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Msg {
    Move(i32),
    Page(i32),
    ToTop,
    ToBottom,
    Select(usize),
    Loaded(usize),
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutMsg {
    StepRequested(usize),
}

impl CommitPicker {
    pub fn empty() -> Self {
        Self {
            list: CommitList::Empty,
        }
    }

    pub fn steps(steps: Arc<[CommitStep]>) -> Self {
        if steps.is_empty() {
            return Self::empty();
        }
        Self {
            list: CommitList::Steps {
                steps,
                cursor: 0,
                scroll: 0,
                target: None,
            },
        }
    }

    pub fn steps_slice(&self) -> &[CommitStep] {
        match &self.list {
            CommitList::Empty => &[],
            CommitList::Steps { steps, .. } => steps,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.list, CommitList::Empty)
    }

    pub fn cursor(&self) -> usize {
        match &self.list {
            CommitList::Empty => 0,
            CommitList::Steps { cursor, .. } => *cursor,
        }
    }

    pub fn scroll(&self) -> usize {
        match &self.list {
            CommitList::Empty => 0,
            CommitList::Steps { scroll, .. } => *scroll,
        }
    }

    pub fn target(&self) -> Option<usize> {
        match &self.list {
            CommitList::Empty => None,
            CommitList::Steps { target, .. } => *target,
        }
    }

    pub fn navigation_index(&self) -> usize {
        self.target().unwrap_or_else(|| self.cursor())
    }

    pub fn set_scroll(&mut self, scroll: usize) {
        if let CommitList::Steps { scroll: slot, .. } = &mut self.list {
            *slot = scroll;
        }
    }

    fn len(&self) -> usize {
        self.steps_slice().len()
    }

    pub fn update(&mut self, msg: Msg) -> Option<OutMsg> {
        match msg {
            Msg::Move(delta) => {
                let next = self.offset_by(delta);
                self.select(next)
            }
            Msg::Page(delta) => {
                let next = self.offset_by(delta);
                self.select(next)
            }
            Msg::ToTop => self.select(0),
            Msg::ToBottom => self.select(self.len().saturating_sub(1)),
            Msg::Select(index) => self.select(index),
            Msg::Loaded(index) => {
                let len = self.len();
                if let CommitList::Steps { cursor, target, .. } = &mut self.list {
                    *cursor = index.min(len.saturating_sub(1));
                    *target = None;
                }
                None
            }
            Msg::Failed => {
                if let CommitList::Steps { target, .. } = &mut self.list {
                    *target = None;
                }
                None
            }
        }
    }

    fn offset_by(&self, delta: i32) -> usize {
        let len = self.len();
        if len == 0 {
            return 0;
        }
        let current = self.navigation_index();
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            current.saturating_add(delta as usize)
        };
        next.min(len - 1)
    }

    fn select(&mut self, index: usize) -> Option<OutMsg> {
        if self.is_empty() {
            return None;
        }
        let index = index.min(self.len() - 1);
        let cursor = self.cursor();
        if index == cursor {
            if let CommitList::Steps { target, .. } = &mut self.list {
                *target = None;
            }
            return None;
        }
        if let CommitList::Steps { target, .. } = &mut self.list {
            *target = Some(index);
        }
        Some(OutMsg::StepRequested(index))
    }
}

pub(crate) fn render(
    state: &CommitPicker,
    ctx: &RenderCtx,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    edge: Edge,
) {
    let theme = ctx.theme;
    let block = pane_block(" Commits ", focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let steps = state.steps_slice();
    if steps.is_empty() {
        let (title, detail) = if ctx.busy {
            ("Loading", "Loading commits…")
        } else {
            ("No commits", "No commits in the selected range.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    }

    let cursor = state.cursor();
    let offset = commit_offset(cursor, state.scroll(), steps.len(), inner.height as usize);
    let mut lines = Vec::new();
    for (index, step) in steps
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner.height as usize)
    {
        let selected = index == cursor;
        let style = if selected {
            theme.fg_bg(theme.palette.selection_fg, theme.palette.selection_bg)
        } else {
            theme.fg(theme.palette.text)
        };
        let id = step.commit_id.to_string();
        let position = format!(" {}/{}", index + 1, steps.len());
        let prefix = format!(
            "{} {} ",
            if selected { "▌" } else { " " },
            &id[..id.len().min(7)]
        );
        let used = prefix.len() + position.len();
        let subject = truncate_ellipsis(&step.subject, (inner.width as usize).saturating_sub(used));
        let mut row = format!("{prefix}{subject}");
        let padding = (inner.width as usize)
            .saturating_sub(unicode_width::UnicodeWidthStr::width(row.as_str()) + position.len());
        row.push_str(&" ".repeat(padding));
        row.push_str(&position);
        lines.push(Line::from(Span::styled(
            row,
            if selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            },
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
