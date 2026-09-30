//! Layout arithmetic shared by the view, the derived-layout cache, and mouse
//! hit-testing: where the panes sit, how a diff wraps into visual rows, and the
//! offsets that keep a cursor visible.
//!
//! Everything here is pure. It reads no terminal and no model behaviour; the
//! values it needs are passed in. The one exception is [`Focus`] and the
//! responsive width constants, which live in [`crate::app`] because they are
//! app state.

use base::{AlignedRow, DiffRowKind, FileDiff};
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{Focus, SIDE_BY_SIDE_MIN_WIDTH};
use crate::highlight::{self, Run, StyledLine};
use crate::theme::Theme;

// Splits a terminal of `width` × `height` into header, content, and status.
// Pure, so `update` and `view` agree on where the content body sits.
pub(crate) fn frame_areas(width: u16, height: u16) -> (Rect, Rect, Rect) {
    let area = Rect::new(0, 0, width, height);
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    (chunks[0], chunks[1], chunks[2])
}

// Where a column sits among its neighbours, which decides which borders it
// draws so adjacent panes share a single divider instead of doubling it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Edge {
    Solo,
    Left,
    Middle,
    Right,
}

// Splits `area` into content columns separated by single-column dividers.
//
// `weights` are relative widths. Returns `(columns, dividers)`, where
// `dividers` has one fewer entry than `columns`. When there is not enough room
// for every divider, they are dropped and the columns share the space.
pub(crate) fn split_with_dividers(area: Rect, weights: &[u16]) -> (Vec<Rect>, Vec<Rect>) {
    let count = weights.len();
    if count == 0 {
        return (Vec::new(), Vec::new());
    }
    if count == 1 {
        return (vec![area], Vec::new());
    }

    let total_weight: u32 = weights.iter().map(|&w| u32::from(w.max(1))).sum();
    let divider_count = count - 1;
    // Drop dividers if the area is too narrow to keep every column non-empty.
    let dividers = if area.width as usize > divider_count + count {
        divider_count
    } else {
        0
    };
    let available = u32::from(area.width) - dividers as u32;

    let mut widths = Vec::with_capacity(count);
    let mut assigned = 0u32;
    for (index, &weight) in weights.iter().enumerate() {
        let width = if index + 1 == count {
            available - assigned
        } else {
            available * u32::from(weight.max(1)) / total_weight
        };
        widths.push(width as u16);
        assigned += width;
    }

    let mut columns = Vec::with_capacity(count);
    let mut divider_rects = Vec::with_capacity(dividers);
    let mut x = area.x;
    for (index, width) in widths.iter().enumerate() {
        columns.push(Rect {
            x,
            y: area.y,
            width: *width,
            height: area.height,
        });
        x = x.saturating_add(*width);
        if index < dividers {
            divider_rects.push(Rect {
                x,
                y: area.y,
                width: 1,
                height: area.height,
            });
            x = x.saturating_add(1);
        }
    }
    (columns, divider_rects)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneSlot {
    Commits,
    Tree,
    Show,
    Old,
    New,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot {
    pub pane: PaneSlot,
    pub outer: Rect,
    pub edge: Edge,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BodyLayout {
    pub slots: Vec<Slot>,
    pub dividers: Vec<Rect>,
}

// The body panes for the current size, content, and focus.
//
// Shared by `view::render_body` and mouse hit-testing, so a click or wheel maps
// to exactly the pane that was drawn. Below [`SIDE_BY_SIDE_MIN_WIDTH`] only the
// focused pane occupies the body; a focused diff stacks old over new. When the
// tree is focused in a narrow terminal every pane is hidden except the tree.
pub(crate) fn body_layout(
    content: Rect,
    tree_percent: u16,
    is_diff: bool,
    has_commits: bool,
    focus: Focus,
) -> BodyLayout {
    if content.width < SIDE_BY_SIDE_MIN_WIDTH {
        let slots = match (is_diff, focus) {
            (true, Focus::Commits) if has_commits => vec![Slot {
                pane: PaneSlot::Commits,
                outer: content,
                edge: Edge::Solo,
            }],
            (_, Focus::Tree) => vec![Slot {
                pane: PaneSlot::Tree,
                outer: content,
                edge: Edge::Solo,
            }],
            (false, _) => vec![Slot {
                pane: PaneSlot::Show,
                outer: content,
                edge: Edge::Solo,
            }],
            (true, _) => {
                let halves =
                    Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(content);
                vec![
                    Slot {
                        pane: PaneSlot::Old,
                        outer: halves[0],
                        edge: Edge::Solo,
                    },
                    Slot {
                        pane: PaneSlot::New,
                        outer: halves[1],
                        edge: Edge::Solo,
                    },
                ]
            }
        };
        return BodyLayout {
            slots,
            dividers: Vec::new(),
        };
    }

    let (panes, weights): (Vec<PaneSlot>, Vec<u16>) = if is_diff {
        let rest = 100 - tree_percent;
        let side = rest / 2;
        (
            vec![PaneSlot::Tree, PaneSlot::Old, PaneSlot::New],
            vec![tree_percent, side, rest - side],
        )
    } else {
        (
            vec![PaneSlot::Tree, PaneSlot::Show],
            vec![tree_percent, 100 - tree_percent],
        )
    };
    let (columns, dividers) = split_with_dividers(content, &weights);
    let last = panes.len() - 1;
    let mut slots: Vec<Slot> = panes
        .into_iter()
        .enumerate()
        .map(|(index, pane)| Slot {
            pane,
            outer: columns[index],
            edge: edge_for(index, last),
        })
        .collect();
    if has_commits && is_diff {
        let left = slots.remove(0);
        // Leave at least three rows for Files. At normal sizes the picker has
        // seven visible rows plus its borders, and shrinks on short terminals.
        let commits_height = left
            .outer
            .height
            .min(9)
            .min(left.outer.height.saturating_sub(3));
        let commits = Slot {
            pane: PaneSlot::Commits,
            outer: Rect {
                height: commits_height,
                ..left.outer
            },
            edge: left.edge,
        };
        let files = Slot {
            pane: PaneSlot::Tree,
            outer: Rect {
                y: left.outer.y + commits_height,
                height: left.outer.height - commits_height,
                ..left.outer
            },
            edge: left.edge,
        };
        slots.insert(0, files);
        slots.insert(0, commits);
    }
    BodyLayout { slots, dividers }
}

pub(crate) fn commit_offset(cursor: usize, scroll: usize, len: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    let max = len.saturating_sub(height);
    let scroll = scroll.min(max);
    if cursor < scroll {
        cursor
    } else if cursor >= scroll.saturating_add(height) {
        cursor.saturating_add(1).saturating_sub(height).min(max)
    } else {
        scroll
    }
}

// The border edge a column draws, so adjacent panes share one divider.
fn edge_for(index: usize, last: usize) -> Edge {
    match (index, last) {
        (0, 0) => Edge::Solo,
        (0, _) => Edge::Left,
        (index, last) if index == last => Edge::Right,
        _ => Edge::Middle,
    }
}

pub(crate) fn gutter_width(diff: &FileDiff) -> usize {
    let lines = |file: &base::ProjectedFile| file.canonical_text().lines().count();
    let max_lines = match diff {
        FileDiff::Added { new } => lines(new),
        FileDiff::Deleted { old } => lines(old),
        FileDiff::Modified { old, new } => lines(old).max(lines(new)),
    };
    max_lines.max(1).to_string().len() + 2
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

pub(crate) fn window_offset(cursor: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height || cursor < height {
        0
    } else {
        (cursor + 1).saturating_sub(height)
    }
}

// ---------------------------------------------------------------------------
// Wrapped diff layout
// ---------------------------------------------------------------------------

/// The kind of a rendered diff row: either an aligned diff row or a synthetic
/// hunk header inserted where context was collapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowKind {
    Diff(DiffRowKind),
    Hunk,
    /// A gap between hunks: a count of unchanged rows that were collapsed.
    Collapse(usize),
}

/// One visual row of a wrapped side-by-side diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub kind: VisualRowKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_runs: StyledLine,
    pub new_runs: StyledLine,
    pub continuation: bool,
}

// Context lines kept around each change, matching the core diff engine.
const CONTEXT_RADIUS: usize = 3;

// Wraps one logical aligned row into visual rows for both panes.
//
// Each side wraps independently to its own content width; the row occupies the
// greater height and the shorter side is padded with blank rows so later rows
// stay aligned. Only the first visual row carries line numbers. Changed rows
// also get delta-style intra-line emphasis on the bytes that differ.
pub(crate) fn layout_diff(
    diff: &FileDiff,
    old_width: usize,
    new_width: usize,
    old_highlight: &[StyledLine],
    new_highlight: &[StyledLine],
    theme: &Theme,
) -> Vec<VisualRow> {
    let (old_text, new_text) = match diff {
        FileDiff::Added { new } => ("", new.canonical_text()),
        FileDiff::Deleted { old } => (old.canonical_text(), ""),
        FileDiff::Modified { old, new } => (old.canonical_text(), new.canonical_text()),
    };
    let rows = base::aligned_rows(old_text, new_text);

    let mut visual = Vec::new();
    let mut previous_end: Option<usize> = None;
    for (start, end) in context_windows(&rows) {
        if let Some(previous_end) = previous_end {
            let hidden = start.saturating_sub(previous_end + 1);
            if hidden > 0 {
                visual.push(collapse_row(hidden));
            }
        }
        visual.push(hunk_header(&rows, start, end));
        for row in &rows[start..=end] {
            let (old, new, old_emphasis, new_emphasis) = match row {
                AlignedRow::Equal { old, new } => (Some(old), Some(new), Vec::new(), Vec::new()),
                AlignedRow::Add { new } => (None, Some(new), Vec::new(), Vec::new()),
                AlignedRow::Delete { old } => (Some(old), None, Vec::new(), Vec::new()),
                AlignedRow::Change { old, new } => {
                    let (old_emphasis, new_emphasis) =
                        highlight::emphasis_ranges(old.text, new.text);
                    (Some(old), Some(new), old_emphasis, new_emphasis)
                }
            };

            let old_segments = old
                .map(|line| {
                    let runs = styled_line(old_highlight, line.number, line.text);
                    let runs = highlight::apply_emphasis(
                        &runs,
                        &old_emphasis,
                        theme.color(theme.palette.del_emph),
                    );
                    highlight::wrap_runs(&runs, old_width)
                })
                .unwrap_or_default();
            let new_segments = new
                .map(|line| {
                    let runs = styled_line(new_highlight, line.number, line.text);
                    let runs = highlight::apply_emphasis(
                        &runs,
                        &new_emphasis,
                        theme.color(theme.palette.add_emph),
                    );
                    highlight::wrap_runs(&runs, new_width)
                })
                .unwrap_or_default();

            let height = old_segments.len().max(new_segments.len()).max(1);
            for index in 0..height {
                visual.push(VisualRow {
                    kind: VisualRowKind::Diff(row.kind()),
                    old_number: if index == 0 {
                        old.map(|line| line.number)
                    } else {
                        None
                    },
                    new_number: if index == 0 {
                        new.map(|line| line.number)
                    } else {
                        None
                    },
                    old_runs: old_segments.get(index).cloned().unwrap_or_default(),
                    new_runs: new_segments.get(index).cloned().unwrap_or_default(),
                    continuation: index > 0,
                });
            }
        }
        previous_end = Some(end);
    }
    visual
}

fn collapse_row(hidden: usize) -> VisualRow {
    let text = format!("⋯ {hidden} unchanged lines");
    let run = Run {
        style: ratatui::style::Style::default(),
        text,
    };
    VisualRow {
        kind: VisualRowKind::Collapse(hidden),
        old_number: None,
        new_number: None,
        old_runs: vec![run.clone()],
        new_runs: vec![run],
        continuation: false,
    }
}

// The inclusive index ranges of aligned rows to display, one per hunk.
//
// Each change pulls in [`CONTEXT_RADIUS`] rows of surrounding context; ranges
// that touch or overlap merge, and the gaps between them become hunk headers.
fn context_windows(rows: &[AlignedRow<'_>]) -> Vec<(usize, usize)> {
    let changes: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.kind() != DiffRowKind::Equal)
        .map(|(index, _)| index)
        .collect();

    if changes.is_empty() {
        return if rows.is_empty() {
            Vec::new()
        } else {
            vec![(0, rows.len() - 1)]
        };
    }

    let mut windows: Vec<(usize, usize)> = Vec::new();
    for index in changes {
        let start = index.saturating_sub(CONTEXT_RADIUS);
        let end = (index + CONTEXT_RADIUS).min(rows.len() - 1);
        match windows.last_mut() {
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => windows.push((start, end)),
        }
    }
    windows
}

fn hunk_header(rows: &[AlignedRow<'_>], start: usize, end: usize) -> VisualRow {
    let slice = &rows[start..=end];
    let old_numbers: Vec<usize> = slice
        .iter()
        .filter_map(|row| match row {
            AlignedRow::Equal { old, .. }
            | AlignedRow::Delete { old }
            | AlignedRow::Change { old, .. } => Some(old.number),
            AlignedRow::Add { .. } => None,
        })
        .collect();
    let new_numbers: Vec<usize> = slice
        .iter()
        .filter_map(|row| match row {
            AlignedRow::Equal { new, .. }
            | AlignedRow::Add { new }
            | AlignedRow::Change { new, .. } => Some(new.number),
            AlignedRow::Delete { .. } => None,
        })
        .collect();
    let old_start = old_numbers.first().copied().unwrap_or(0);
    let new_start = new_numbers.first().copied().unwrap_or(0);
    let text = format!(
        "@@ -{old_start},{} +{new_start},{} @@",
        old_numbers.len(),
        new_numbers.len()
    );
    let run = Run {
        style: ratatui::style::Style::default(),
        text,
    };
    VisualRow {
        kind: VisualRowKind::Hunk,
        old_number: None,
        new_number: None,
        old_runs: vec![run.clone()],
        new_runs: vec![run],
        continuation: false,
    }
}

fn styled_line(highlight: &[StyledLine], number: usize, text: &str) -> StyledLine {
    match highlight.get(number.saturating_sub(1)) {
        Some(line) if !line.is_empty() => line.clone(),
        _ => vec![Run {
            style: ratatui::style::Style::default(),
            text: text.to_owned(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use base::{ItemKind, ProjectedFile, ProjectedItem, RepoPath, SourceSpan, SupportedPath};

    fn content() -> Rect {
        Rect::new(0, 1, 100, 28)
    }

    #[test]
    fn a_show_splits_into_a_tree_and_a_content_pane() {
        let layout = body_layout(content(), 30, false, false, Focus::Tree);
        assert_eq!(layout.slots.len(), 2);
        assert_eq!(layout.slots[0].pane, PaneSlot::Tree);
        assert_eq!(layout.slots[0].edge, Edge::Left);
        assert_eq!(layout.slots[1].pane, PaneSlot::Show);
        assert_eq!(layout.slots[1].edge, Edge::Right);
        assert_eq!(layout.dividers.len(), 1);
        // The two panes and the divider tile the content with no overlap.
        assert_eq!(
            layout.slots[0].outer.width + 1 + layout.slots[1].outer.width,
            content().width
        );
    }

    #[test]
    fn a_diff_splits_into_tree_old_and_new() {
        let layout = body_layout(content(), 30, true, false, Focus::Content);
        let panes: Vec<PaneSlot> = layout.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![PaneSlot::Tree, PaneSlot::Old, PaneSlot::New]);
        assert_eq!(layout.slots[1].edge, Edge::Middle);
        assert_eq!(layout.slots[2].edge, Edge::Right);
        assert_eq!(layout.dividers.len(), 2);
    }

    #[test]
    fn a_narrow_body_shows_only_the_focused_pane() {
        let narrow = Rect::new(0, 1, 60, 20);

        let tree = body_layout(narrow, 30, false, false, Focus::Tree);
        assert_eq!(tree.slots.len(), 1);
        assert_eq!(tree.slots[0].pane, PaneSlot::Tree);
        assert!(tree.dividers.is_empty());

        let body = body_layout(narrow, 30, false, false, Focus::Content);
        assert_eq!(body.slots[0].pane, PaneSlot::Show);

        // A focused diff stacks old over new, tiling the content vertically.
        let diff = body_layout(narrow, 30, true, false, Focus::Content);
        let panes: Vec<PaneSlot> = diff.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![PaneSlot::Old, PaneSlot::New]);
        assert_eq!(diff.slots[0].outer.height + diff.slots[1].outer.height, 20);
    }

    #[test]
    fn commits_split_only_the_left_column_and_adapt_to_height() {
        let normal = body_layout(content(), 30, true, true, Focus::Commits);
        let panes: Vec<_> = normal.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(
            panes,
            vec![
                PaneSlot::Commits,
                PaneSlot::Tree,
                PaneSlot::Old,
                PaneSlot::New
            ]
        );
        assert_eq!(normal.slots[0].outer.height, 9);
        assert_eq!(normal.slots[1].outer.height, content().height - 9);
        assert_eq!(normal.slots[0].outer.width, normal.slots[1].outer.width);
        assert_eq!(normal.slots[2].outer.height, content().height);

        let short = body_layout(Rect::new(0, 1, 100, 6), 30, true, true, Focus::Commits);
        assert_eq!(short.slots[0].outer.height, 3);
        assert_eq!(short.slots[1].outer.height, 3);

        let narrow = Rect::new(0, 1, 60, 20);
        for (focus, expected) in [
            (Focus::Commits, PaneSlot::Commits),
            (Focus::Tree, PaneSlot::Tree),
        ] {
            let layout = body_layout(narrow, 30, true, true, focus);
            assert_eq!(layout.slots.len(), 1);
            assert_eq!(layout.slots[0].pane, expected);
            assert_eq!(layout.slots[0].outer, narrow);
        }
    }

    #[test]
    fn commit_offset_keeps_selected_row_visible() {
        assert_eq!(commit_offset(0, 0, 12, 7), 0);
        assert_eq!(commit_offset(7, 0, 12, 7), 1);
        assert_eq!(commit_offset(11, 1, 12, 7), 5);
        assert_eq!(commit_offset(2, 5, 12, 7), 2);
        assert_eq!(commit_offset(2, 5, 12, 0), 0);
    }

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path =
            SupportedPath::new(RepoPath::new(path).expect("valid path")).expect("supported path");
        ProjectedFile::try_new(
            path,
            vec![ProjectedItem {
                stable_key: "item".to_owned(),
                parent_key: None,
                kind: ItemKind::Function,
                name: "item".to_owned(),
                span: SourceSpan::new(0, 0, 0, 0, 0, 0),
                canonical_text: text.to_owned(),
            }],
        )
        .expect("valid fixture")
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
}
