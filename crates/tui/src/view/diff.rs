use base::{AlignedRow, DiffRowKind, FileDiff};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, VisualRow, VisualRowKind};
use crate::content::{Loaded, SearchSide};
use crate::highlight::{self, Run, StyledLine};
use crate::theme::Theme;

use super::empty::render_empty;
use super::geom::{Edge, gutter_width, pane_block};
use super::text::truncate_ellipsis;

// Context lines kept around each change, matching the core diff engine.
const CONTEXT_RADIUS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Side {
    Old,
    New,
}

pub(crate) fn render_diff_pane(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    side: Side,
    focused: bool,
    rows: &[VisualRow],
    edge: Edge,
) {
    let theme = &app.chrome.theme;
    let Loaded::Diff(diff) = &app.loaded else {
        return;
    };
    let (base, target) = diff.revisions();
    let revision = match side {
        Side::Old => base,
        Side::New => target,
    };
    let title = pane_title(side, &revision, area.width);
    let block = pane_block(&title, focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(active) = diff.active_diff() else {
        let (title, detail) = if app.is_busy() && app.tree.rows.is_empty() {
            ("Loading", "Comparing projections…")
        } else if app.tree.rows.is_empty() {
            ("No changes", "Body-only edits are omitted.")
        } else {
            ("No file selected", "Select a file to view its diff.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    };
    if rows.is_empty() {
        render_empty(
            frame,
            inner,
            "No text changes",
            "No projected text to compare for this file.",
            theme,
        );
        return;
    }
    let gutter = gutter_width(active);
    let content_width = (inner.width as usize).saturating_sub(gutter);
    let height = inner.height as usize;
    let skip = diff.body_scroll() as usize;

    let mut lines = Vec::new();
    for row in rows.iter().skip(skip).take(height) {
        let (number, runs) = match side {
            Side::Old => (row.old_number, row.old_runs.as_slice()),
            Side::New => (row.new_number, row.new_runs.as_slice()),
        };
        let ranges = if row.continuation {
            Vec::new()
        } else {
            number.map_or_else(Vec::new, |line| diff.search_ranges(search_side(side), line))
        };
        lines.push(diff_line(
            number,
            row.continuation,
            runs,
            row.kind,
            side,
            gutter,
            content_width,
            theme,
            &ranges,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn pane_title(side: Side, revision: &str, pane_width: u16) -> String {
    let label = match side {
        Side::Old => "BASE",
        Side::New => "TARGET",
    };
    // Leave room for the rounded border corners on a standalone pane. This
    // also keeps the title within the border in shared-divider layouts.
    let available = (pane_width as usize).saturating_sub(2);
    truncate_ellipsis(&format!(" {label} · {revision} "), available)
}

fn search_side(side: Side) -> SearchSide {
    match side {
        Side::Old => SearchSide::Old,
        Side::New => SearchSide::New,
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_line(
    number: Option<usize>,
    continuation: bool,
    runs: &[Run],
    kind: VisualRowKind,
    side: Side,
    gutter: usize,
    width: usize,
    theme: &Theme,
    search: &[(usize, usize, bool)],
) -> Line<'static> {
    let VisualRowKind::Diff(kind) = kind else {
        // Hunk headers and collapse indicators span the whole pane.
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        let full = gutter + width;
        let mut padded = if kind == VisualRowKind::Hunk {
            format!(" {text}")
        } else {
            text
        };
        let used = UnicodeWidthStr::width(padded.as_str());
        if used < full {
            padded.push_str(&" ".repeat(full - used));
        }
        let style = match kind {
            VisualRowKind::Hunk if theme.colors_enabled() => theme
                .fg_bg(theme.palette.hunk, theme.palette.surface_alt)
                .add_modifier(Modifier::BOLD),
            VisualRowKind::Hunk => Style::default(),
            _ => theme.fg(theme.palette.text_muted),
        };
        return Line::from(Span::styled(padded, style));
    };

    // Delta-style full-line background; the intra-line emphasis is already
    // baked into the run styles, so a run's own background wins over the base.
    let base = line_background(kind, side, theme);

    // Search matches are recolored on top of the diff emphasis.
    let owned;
    let runs: &[Run] = if search.is_empty() {
        runs
    } else {
        let mut styled = runs.to_vec();
        for (start, end, current) in search {
            let color = if *current {
                theme.color(theme.palette.match_current_bg)
            } else {
                theme.color(theme.palette.match_bg)
            };
            styled = highlight::apply_emphasis(&styled, &[(*start, *end)], color);
        }
        owned = styled;
        &owned
    };

    let field = gutter.saturating_sub(2);
    let sign = match (kind, side) {
        (DiffRowKind::Add, Side::New) | (DiffRowKind::Change, Side::New) => "+",
        (DiffRowKind::Delete, Side::Old) | (DiffRowKind::Change, Side::Old) => "-",
        _ => " ",
    };
    let number = match number {
        Some(number) => format!("{number:>field$}"),
        None if continuation => format!("{:>field$}", "…"),
        None => " ".repeat(field),
    };
    let sign_color = match sign {
        "+" => theme.palette.add_fg,
        "-" => theme.palette.del_fg,
        _ => theme.palette.gutter,
    };
    let mut marker_style = theme.fg(sign_color);
    if marker_style.bg.is_none() {
        marker_style.bg = base.bg;
    }
    let mut spans = vec![Span::styled(format!("{sign}{number} "), marker_style)];
    let mut used = 0usize;
    for run in runs {
        used += UnicodeWidthStr::width(run.text.as_str());
        let mut style = run.style;
        if style.bg.is_none() {
            style.bg = base.bg;
        }
        spans.push(Span::styled(run.text.clone(), style));
    }
    let padding = width.saturating_sub(used);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), base));
    }
    Line::from(spans)
}

fn line_background(kind: DiffRowKind, side: Side, theme: &Theme) -> Style {
    if !theme.colors_enabled() {
        return Style::default();
    }
    match (kind, side) {
        (DiffRowKind::Add, _) | (DiffRowKind::Change, Side::New) => theme.bg(theme.palette.add_bg),
        (DiffRowKind::Delete, _) | (DiffRowKind::Change, Side::Old) => {
            theme.bg(theme.palette.del_bg)
        }
        _ => Style::default(),
    }
}

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
        style: Style::default(),
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
        style: Style::default(),
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
            style: Style::default(),
            text: text.to_owned(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Capability, Flavor};

    use base::{ItemKind, ProjectedFile, ProjectedItem, RepoPath, SourceSpan, SupportedPath};

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
                .all(|row| row.kind == VisualRowKind::Diff(base::DiffRowKind::Add))
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
        assert_eq!(rows[0].kind, VisualRowKind::Diff(base::DiffRowKind::Delete));
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

    #[test]
    fn pane_titles_identify_sides_and_fit_narrow_borders() {
        assert_eq!(pane_title(Side::Old, "main", 20), " BASE · main ");
        assert_eq!(pane_title(Side::New, "feature", 20), " TARGET · feature ");

        let narrow = pane_title(Side::New, "a-long-feature-branch", 16);
        assert_eq!(narrow, " TARGET · a-l…");
        assert_eq!(UnicodeWidthStr::width(narrow.as_str()), 14);
        assert_eq!(pane_title(Side::Old, "main", 2), "");
    }

    #[test]
    fn hunk_header_fills_the_pane_with_a_subtle_band() {
        let theme = Theme::dark();
        let runs = vec![Run {
            style: Style::default(),
            text: "@@ -12,3 +12,4 @@".to_owned(),
        }];

        let line = diff_line(
            None,
            false,
            &runs,
            VisualRowKind::Hunk,
            Side::Old,
            4,
            24,
            &theme,
            &[],
        );
        let span = &line.spans[0];
        assert_eq!(UnicodeWidthStr::width(span.content.as_ref()), 28);
        assert!(span.content.starts_with(" @@ -12,3 +12,4 @@"));
        assert_eq!(span.style.fg, Some(theme.color(theme.palette.hunk)));
        assert_eq!(span.style.bg, Some(theme.color(theme.palette.surface_alt)));
    }

    #[test]
    fn hunk_header_keeps_its_range_markers_without_color() {
        let theme = Theme::new(Flavor::Dark, Capability::NoColor);
        let runs = vec![Run {
            style: Style::default(),
            text: "@@ -1,2 +1,3 @@".to_owned(),
        }];

        let line = diff_line(
            None,
            false,
            &runs,
            VisualRowKind::Hunk,
            Side::New,
            4,
            20,
            &theme,
            &[],
        );
        assert!(line.spans[0].content.starts_with(" @@ -1,2 +1,3 @@"));
        assert_eq!(line.spans[0].style, Style::default());
    }
}
