//! The diff's visual rows: wrapped sides, hunk headers, collapse markers, and
//! the derived side widths.

use base::{AlignedRow, DiffRowKind, FileDiff};

use crate::{DIFF_BODY_FOCUS, NEW_PANE, OLD_PANE};
use tui_core::cx::Cx;
use tui_core::layout::PaneSpec;
use tui_core::render::highlight::{self, Run, StyledLine};
use tui_core::render::theme::Theme;

use super::emphasis::emphasis_ranges;

/// One drawn diff row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowKind {
    Diff(DiffRowKind),
    Hunk,
    Collapse(usize),
}

/// A wrapped row of both diff sides, with its line numbers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualRow {
    pub kind: VisualRowKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub old_runs: StyledLine,
    pub new_runs: StyledLine,
    pub continuation: bool,
}

const CONTEXT_RADIUS: usize = 3;

/// The wrapped width of each diff side, from the body layout so the rows wrap
/// exactly where the panes draw. The body focus is forced so both sides exist
/// even on a narrow terminal focused elsewhere.
pub fn side_widths(cx: &Cx, diff: &FileDiff) -> (usize, usize) {
    let gutter = gutter_width(diff);
    let layout = cx.layout_if_focused(DIFF_BODY_FOCUS);
    let width = |pane| {
        layout
            .slots
            .iter()
            .find(|slot| slot.pane == pane)
            .map_or(0, |slot| slot.geometry.inner.width)
    };
    (
        (width(OLD_PANE) as usize).saturating_sub(gutter),
        (width(NEW_PANE) as usize).saturating_sub(gutter),
    )
}

/// The two body panes of a diff view, in registration order.
pub fn diff_panes(has_commits: bool) -> Vec<PaneSpec> {
    let mut panes = vec![super::plugin::OLD_SPEC, super::plugin::NEW_SPEC];
    if has_commits {
        panes.push(super::plugin::COMMITS_SPEC);
    }
    panes
}

pub fn commit_offset(cursor: usize, scroll: usize, len: usize, height: usize) -> usize {
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

pub fn gutter_width(diff: &FileDiff) -> usize {
    let lines = |file: &base::ProjectedFile| file.canonical_text().lines().count();
    let max_lines = match diff {
        FileDiff::Added { new } => lines(new),
        FileDiff::Deleted { old } => lines(old),
        FileDiff::Modified { old, new } => lines(old).max(lines(new)),
    };
    max_lines.max(1).to_string().len() + 2
}

pub fn layout_diff(
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
                    let (old_emphasis, new_emphasis) = emphasis_ranges(old.text, new.text);
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

    use crate::{COMMITS_PANE, DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, NEW_PANE, OLD_PANE};
    use ratatui::layout::Rect;
    use tui_core::layout::SIDE_BY_SIDE_MIN_WIDTH;
    use tui_core::layout::split_with_dividers;
    use tui_core::layout::{Edge, PaneSpec, Placement};
    use tui_core::plugin::{FocusKey, PaneId, PluginId, Size};
    use tui_core::render::block::pane_block;
    use tui_core::render::icons::{IconStyle, Icons};

    const TREE_PANE: PaneId = PaneId {
        owner: PluginId("fake-tree"),
        slot: 0,
    };
    const TREE_FOCUS: FocusKey = FocusKey {
        owner: PluginId("fake-tree"),
        slot: 0,
    };

    const TREE: PaneSpec = PaneSpec {
        id: TREE_PANE,
        placement: Placement::Navigator,
        focus: TREE_FOCUS,
    };
    const OLD: PaneSpec = PaneSpec {
        id: OLD_PANE,
        placement: Placement::Body,
        focus: DIFF_BODY_FOCUS,
    };
    const NEW: PaneSpec = PaneSpec {
        id: NEW_PANE,
        placement: Placement::Body,
        focus: DIFF_BODY_FOCUS,
    };
    const COMMITS: PaneSpec = PaneSpec {
        id: COMMITS_PANE,
        placement: Placement::Band,
        focus: DIFF_BAND_FOCUS,
    };

    fn with_width<R>(width: u16, panes: &[PaneSpec], f: impl FnOnce(&Cx) -> R) -> R {
        let theme = Theme::dark();
        let icons = Icons::new(IconStyle::None);
        let workspace = tui_core::workspace::Workspace {
            query: tui_core::workspace::Query {
                mode: base::ProjectionMode::Types,
                scope: tui_core::workspace::Scope::new(base::Selection::all()),
            },
            files: tui_core::workspace::FileSet::default(),
            selected: None,
            search: None,
        };
        let cx = tui_core::testing::cx_with(
            Size { width, height: 30 },
            &theme,
            &icons,
            &workspace,
            Some(DIFF_BODY_FOCUS),
            crate::COMPARE_VIEW,
            panes,
            &[],
            &[],
        );
        f(&cx)
    }

    /// The production formula `side_widths` replaced. Kept only so the sweep
    /// test can compare the layout-derived widths against it.
    fn legacy_side_widths(
        width: u16,
        content: Rect,
        tree_percent: u16,
        diff: &FileDiff,
        theme: &Theme,
    ) -> (usize, usize) {
        let gutter = gutter_width(diff);
        if width >= SIDE_BY_SIDE_MIN_WIDTH {
            let rest = 100 - tree_percent;
            let side = rest / 2;
            let (columns, _) = split_with_dividers(content, &[tree_percent, side, rest - side]);
            let old_inner = pane_block("", false, theme, Edge::Middle).inner(columns[1]);
            let new_inner = pane_block("", false, theme, Edge::Right).inner(columns[2]);
            (
                (old_inner.width as usize).saturating_sub(gutter),
                (new_inner.width as usize).saturating_sub(gutter),
            )
        } else {
            let inner = pane_block("", false, theme, Edge::Solo).inner(content);
            let width = (inner.width as usize).saturating_sub(gutter);
            (width, width)
        }
    }

    #[test]
    fn layout_side_widths_match_the_legacy_formula_across_a_width_sweep() {
        let theme = Theme::dark();
        let small = file_diff("a.rs", Some("x\n"), Some("y\n"));
        let lines: String = (0..120).map(|index| format!("line {index}\n")).collect();
        let changed = lines.replace("line 0", "changed");
        let large = file_diff("b.rs", Some(&lines), Some(&changed));
        assert_ne!(
            gutter_width(&small),
            gutter_width(&large),
            "the sweep needs at least two distinct gutters"
        );

        let range_panes = [TREE, OLD, NEW];
        let commit_panes = [TREE, OLD, NEW, COMMITS];
        for diff in [&small, &large] {
            for width in 40u16..=200 {
                let (_, content, _) = tui_core::layout::frame_areas(width, 30);
                let legacy = legacy_side_widths(width, content, 30, diff, &theme);
                let range = with_width(width, &range_panes, |cx| side_widths(cx, diff));
                let commits = with_width(width, &commit_panes, |cx| side_widths(cx, diff));
                assert_eq!(range, legacy, "range widths at width {width}");
                assert_eq!(commits, legacy, "commits widths at width {width}");
            }
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
