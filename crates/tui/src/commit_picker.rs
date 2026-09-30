//! The `commits` view's picker: a scrollable list of first-parent steps.
//!
//! It owns the cursor, the scroll offset, and the navigation target (the step
//! whose projection is being loaded while the displayed diff still belongs to
//! the previously loaded step). The list and cursor cannot outlive each other:
//! an empty list is a distinct variant with no cursor to dangle.

use std::sync::Arc;

use engine::CommitStep;

/// The steps a commits view selects between.
#[derive(Clone, Debug)]
pub enum CommitList {
    /// No commits in the selected range.
    Empty,
    /// A non-empty list with a cursor, a scroll offset, and an optional target
    /// whose projection is in flight.
    Steps {
        steps: Arc<[CommitStep]>,
        cursor: usize,
        scroll: usize,
        target: Option<usize>,
    },
}

/// The picker's state.
#[derive(Clone, Debug)]
pub struct CommitPicker {
    pub list: CommitList,
}

/// A message the picker understands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Msg {
    /// Move the cursor by a signed number of steps.
    Move(i32),
    /// Page the cursor by a signed number of steps.
    Page(i32),
    ToTop,
    ToBottom,
    /// Select (or begin loading) the step at an absolute index.
    Select(usize),
    /// The in-flight step finished loading; commit it as the shown step.
    Loaded(usize),
    /// A step load failed; drop the target so the shown step stays put.
    Failed,
}

/// What the picker asks its parent to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutMsg {
    /// The step at this index should be loaded.
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

    /// The steps, or an empty slice when there are none.
    pub fn steps_slice(&self) -> &[CommitStep] {
        match &self.list {
            CommitList::Empty => &[],
            CommitList::Steps { steps, .. } => steps,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.list, CommitList::Empty)
    }

    /// The index of the step whose projection is currently shown.
    pub fn cursor(&self) -> usize {
        match &self.list {
            CommitList::Empty => 0,
            CommitList::Steps { cursor, .. } => *cursor,
        }
    }

    /// The scroll offset of the picker.
    pub fn scroll(&self) -> usize {
        match &self.list {
            CommitList::Empty => 0,
            CommitList::Steps { scroll, .. } => *scroll,
        }
    }

    /// The index whose projection is being loaded, if any.
    pub fn target(&self) -> Option<usize> {
        match &self.list {
            CommitList::Empty => None,
            CommitList::Steps { target, .. } => *target,
        }
    }

    /// The index navigation should act on: the target when a load is in flight,
    /// otherwise the shown step.
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

    /// Applies a message. Only a navigation change produces an [`OutMsg`].
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
            // Navigating back to the shown step cancels any in-flight load.
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
