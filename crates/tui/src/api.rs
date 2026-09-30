//! The effect vocabulary: [`Cmd`] describes work the runtime performs, and
//! [`HistoryPayload`] is the data a commits-view load returns.
//!
//! I/O is data. `update` constructs a `Cmd` and never performs it; the runtime
//! in `lib.rs` interprets it against the engine and folds the result back into a
//! `Msg`. The interpreter stays in the runtime so no I/O sits below `lib.rs`.

use std::sync::Arc;

use base::FileDiff;
use engine::CommitStep;

use crate::route::{DiffRequest, ShowRequest};

/// A commits-view effect payload: the steps and the first step's diff.
pub type HistoryPayload = (Arc<[CommitStep]>, Arc<[FileDiff]>);

/// An effect the runtime must interpret. I/O is data, never a side effect of
/// `update`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Cmd {
    Show(ShowRequest),
    Diff(DiffRequest),
    Step {
        request: DiffRequest,
        index: usize,
        step: CommitStep,
    },
    LoadAreas,
}
