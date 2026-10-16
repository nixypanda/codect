//! Layout arithmetic shared by the view, the derived-layout cache, and mouse
//! hit-testing: where the panes sit, how a diff wraps into visual rows, and the
//! offsets that keep a cursor visible.
//!
//! Everything here is pure. It reads no terminal and no model behaviour; the
//! values it needs are passed in, including [`Focus`] and the responsive width
//! constants it shares with the shell from [`crate::render::metrics`].
//!
//! The layout tiles its area exactly: adjacent panes share one divider rather
//! than doubling it, and the commits band is pinned to the bottom, capped, and
//! keeps a minimum strip above it. `body_layout` is the single source of truth
//! shared with mouse hit-testing, so a click or wheel maps to the pane that was
//! drawn, and each wrapped diff side keeps its own width with the shorter side
//! padded with blank rows so later rows stay aligned.

use base::{AlignedRow, DiffRowKind, FileDiff};
use ratatui::layout::{Constraint, Layout, Rect};

use crate::plugin::{FocusKey, PaneId};
use crate::render::block::pane_block;
use crate::render::highlight::{self, Run, StyledLine};
use crate::render::metrics::SIDE_BY_SIDE_MIN_WIDTH;
use crate::render::theme::Theme;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    Solo,
    Left,
    Middle,
    Right,
}

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

/// Where a pane sits in the body: the left column, the main area, or the
/// bottom band.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    Navigator,
    Body,
    Band,
}

/// A pane a plugin declares: its id, its placement, and the focus key it
/// answers to. Panes that share a focus key are one stop in the focus ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneSpec {
    pub id: PaneId,
    pub placement: Placement,
    pub focus: FocusKey,
}

/// A laid-out pane: the rect it owns, the rect inside its border and padding,
/// and which border edges it draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaneGeometry {
    pub outer: Rect,
    pub inner: Rect,
    pub edge: Edge,
}

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub pane: PaneId,
    pub focus: FocusKey,
    pub geometry: PaneGeometry,
}

#[derive(Clone, Debug, Default)]
pub struct BodyLayout {
    pub slots: Vec<Slot>,
    pub dividers: Vec<Rect>,
}

const BAND_MAX_HEIGHT: u16 = 9;

const BODY_MIN_HEIGHT: u16 = 3;

fn split_band(content: Rect) -> (Rect, Rect) {
    let band_height = content
        .height
        .min(BAND_MAX_HEIGHT)
        .min(content.height.saturating_sub(BODY_MIN_HEIGHT));
    let top = Rect {
        height: content.height - band_height,
        ..content
    };
    let band = Rect {
        y: content.y + (content.height - band_height),
        height: band_height,
        ..content
    };
    (top, band)
}

fn pane_geometry(outer: Rect, edge: Edge, theme: &Theme) -> PaneGeometry {
    PaneGeometry {
        outer,
        inner: pane_block("", false, theme, edge).inner(outer),
        edge,
    }
}

fn solo_slot(spec: &PaneSpec, outer: Rect, theme: &Theme) -> Slot {
    Slot {
        pane: spec.id,
        focus: spec.focus,
        geometry: pane_geometry(outer, Edge::Solo, theme),
    }
}

/// Split `rest` percent across `count` columns, each even and the last taking
/// the remainder to total exactly `rest`.
fn body_weights(rest: u16, count: usize) -> Vec<u16> {
    if count == 0 {
        return Vec::new();
    }
    let each = rest / count as u16;
    let mut weights = Vec::with_capacity(count);
    let mut assigned = 0;
    for index in 0..count {
        if index + 1 == count {
            weights.push(rest - assigned);
        } else {
            weights.push(each);
            assigned += each;
        }
    }
    weights
}

/// Split `percent` across `count` navigators. One navigator keeps the whole
/// percent; more share it evenly with the last taking the remainder.
fn navigator_weights(percent: u16, count: usize) -> Vec<u16> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![percent];
    }
    let each = percent / count as u16;
    let mut weights = Vec::with_capacity(count);
    let mut assigned = 0;
    for index in 0..count {
        if index + 1 == count {
            weights.push(percent - assigned);
        } else {
            weights.push(each);
            assigned += each;
        }
    }
    weights
}

/// Stack `count` panes vertically in equal percentages. Two uses 50/50 so odd
/// heights round as they always have; one takes the whole strip.
fn stack_bodies(area: Rect, count: usize) -> Vec<Rect> {
    match count {
        0 => Vec::new(),
        1 => vec![area],
        _ => {
            let each = (100 / count as u16).max(1);
            let constraints = vec![Constraint::Percentage(each); count];
            Layout::vertical(constraints).split(area).to_vec()
        }
    }
}

/// Tile the content rect into the visible panes.
///
/// Bands split off the bottom first and stack in registration order. On a
/// narrow terminal only the focused navigator, or the stacked body panes, are
/// shown; on a wide one the navigators and the body panes share columns.
pub(crate) fn body_layout(
    content: Rect,
    navigator_percent: u16,
    panes: &[PaneSpec],
    focus: Option<FocusKey>,
    theme: &Theme,
) -> BodyLayout {
    let navigators: Vec<&PaneSpec> = panes
        .iter()
        .filter(|pane| pane.placement == Placement::Navigator)
        .collect();
    let bodies: Vec<&PaneSpec> = panes
        .iter()
        .filter(|pane| pane.placement == Placement::Body)
        .collect();
    let bands: Vec<&PaneSpec> = panes
        .iter()
        .filter(|pane| pane.placement == Placement::Band)
        .collect();

    let (strip, band_area) = if bands.is_empty() {
        (content, None)
    } else {
        let (top, band) = split_band(content);
        (top, Some(band))
    };

    let mut slots: Vec<Slot> = Vec::new();
    let mut dividers = Vec::new();

    if strip.width < SIDE_BY_SIDE_MIN_WIDTH {
        let navigator = focus.and_then(|key| navigators.iter().find(|pane| pane.focus == key));
        if let Some(navigator) = navigator {
            slots.push(solo_slot(navigator, strip, theme));
        } else {
            let rects = stack_bodies(strip, bodies.len());
            for (pane, rect) in bodies.iter().zip(rects) {
                slots.push(solo_slot(pane, rect, theme));
            }
        }
    } else {
        let mut columns: Vec<&PaneSpec> = Vec::with_capacity(navigators.len() + bodies.len());
        let mut weights: Vec<u16> = Vec::with_capacity(navigators.len() + bodies.len());
        if navigators.is_empty() {
            columns.extend(bodies.iter().copied());
            weights.extend(body_weights(100, bodies.len()));
        } else {
            columns.extend(navigators.iter().copied());
            weights.extend(navigator_weights(navigator_percent, navigators.len()));
            columns.extend(bodies.iter().copied());
            weights.extend(body_weights(100 - navigator_percent, bodies.len()));
        }
        let (rects, divs) = split_with_dividers(strip, &weights);
        let last = columns.len().saturating_sub(1);
        for (index, pane) in columns.iter().enumerate() {
            let edge = edge_for(index, last);
            slots.push(Slot {
                pane: pane.id,
                focus: pane.focus,
                geometry: pane_geometry(rects[index], edge, theme),
            });
        }
        dividers = divs;
    }

    if let Some(band_area) = band_area {
        let rects = stack_bodies(band_area, bands.len());
        for (pane, rect) in bands.iter().zip(rects) {
            slots.push(solo_slot(pane, rect, theme));
        }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowKind {
    Diff(DiffRowKind),
    Hunk,
    Collapse(usize),
}

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

    use crate::plugin::{
        COMMITS_PANE, DIFF_BAND_FOCUS, DIFF_BODY_FOCUS, NEW_PANE, OLD_PANE, SOURCE_FOCUS,
        SOURCE_PANE, TREE_FOCUS, TREE_PANE,
    };

    const TREE: PaneSpec = PaneSpec {
        id: TREE_PANE,
        placement: Placement::Navigator,
        focus: TREE_FOCUS,
    };
    const SOURCE: PaneSpec = PaneSpec {
        id: SOURCE_PANE,
        placement: Placement::Body,
        focus: SOURCE_FOCUS,
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

    fn content() -> Rect {
        Rect::new(0, 1, 100, 28)
    }

    #[test]
    fn a_show_splits_into_a_tree_and_a_content_pane() {
        let layout = body_layout(
            content(),
            30,
            &[TREE, SOURCE],
            Some(TREE_FOCUS),
            &Theme::dark(),
        );
        assert_eq!(layout.slots.len(), 2);
        assert_eq!(layout.slots[0].pane, TREE_PANE);
        assert_eq!(layout.slots[0].focus, TREE_FOCUS);
        assert_eq!(layout.slots[0].geometry.edge, Edge::Left);
        assert_eq!(layout.slots[1].pane, SOURCE_PANE);
        assert_eq!(layout.slots[1].focus, SOURCE_FOCUS);
        assert_eq!(layout.slots[1].geometry.edge, Edge::Right);
        assert_eq!(layout.dividers.len(), 1);
        assert_eq!(
            layout.slots[0].geometry.outer.width + 1 + layout.slots[1].geometry.outer.width,
            content().width
        );
    }

    #[test]
    fn a_diff_splits_into_tree_old_and_new() {
        let layout = body_layout(
            content(),
            30,
            &[TREE, OLD, NEW],
            Some(DIFF_BODY_FOCUS),
            &Theme::dark(),
        );
        let panes: Vec<PaneId> = layout.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![TREE_PANE, OLD_PANE, NEW_PANE]);
        assert_eq!(layout.slots[1].focus, DIFF_BODY_FOCUS);
        assert_eq!(layout.slots[2].focus, DIFF_BODY_FOCUS);
        assert_eq!(layout.slots[1].geometry.edge, Edge::Middle);
        assert_eq!(layout.slots[2].geometry.edge, Edge::Right);
        assert_eq!(layout.dividers.len(), 2);
    }

    #[test]
    fn a_narrow_body_shows_only_the_focused_pane() {
        let narrow = Rect::new(0, 1, 60, 20);

        let tree = body_layout(
            narrow,
            30,
            &[TREE, SOURCE],
            Some(TREE_FOCUS),
            &Theme::dark(),
        );
        assert_eq!(tree.slots.len(), 1);
        assert_eq!(tree.slots[0].pane, TREE_PANE);
        assert_eq!(tree.slots[0].focus, TREE_FOCUS);
        assert!(tree.dividers.is_empty());

        let body = body_layout(
            narrow,
            30,
            &[TREE, SOURCE],
            Some(SOURCE_FOCUS),
            &Theme::dark(),
        );
        assert_eq!(body.slots[0].pane, SOURCE_PANE);
        assert_eq!(body.slots[0].focus, SOURCE_FOCUS);

        let diff = body_layout(
            narrow,
            30,
            &[TREE, OLD, NEW],
            Some(DIFF_BODY_FOCUS),
            &Theme::dark(),
        );
        let panes: Vec<PaneId> = diff.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![OLD_PANE, NEW_PANE]);
        assert_eq!(
            diff.slots[0].geometry.outer.height + diff.slots[1].geometry.outer.height,
            20
        );
    }

    #[test]
    fn commits_form_a_bottom_band_and_adapt_to_height() {
        let specs = [TREE, OLD, NEW, COMMITS];
        let wide = body_layout(content(), 30, &specs, Some(DIFF_BAND_FOCUS), &Theme::dark());
        let panes: Vec<_> = wide.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![TREE_PANE, OLD_PANE, NEW_PANE, COMMITS_PANE]);
        let band = &wide.slots[3];
        assert_eq!(band.focus, DIFF_BAND_FOCUS);
        assert_eq!(band.geometry.edge, Edge::Solo);
        assert_eq!(band.geometry.outer.width, content().width);
        assert_eq!(band.geometry.outer.height, 9);
        assert_eq!(band.geometry.outer.y, content().y + content().height - 9);
        for slot in &wide.slots[..3] {
            assert_eq!(slot.geometry.outer.height, content().height - 9);
        }
        assert_eq!(wide.dividers.len(), 2);
        for divider in &wide.dividers {
            assert_eq!(divider.height, content().height - 9);
        }

        let short = body_layout(
            Rect::new(0, 1, 100, 6),
            30,
            &specs,
            Some(DIFF_BAND_FOCUS),
            &Theme::dark(),
        );
        let panes: Vec<_> = short.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![TREE_PANE, OLD_PANE, NEW_PANE, COMMITS_PANE]);
        assert_eq!(short.slots[3].geometry.outer.height, 3);
        for slot in &short.slots[..3] {
            assert_eq!(slot.geometry.outer.height, 3);
        }

        let narrow = Rect::new(0, 1, 60, 20);
        let commits = body_layout(narrow, 30, &specs, Some(DIFF_BAND_FOCUS), &Theme::dark());
        let panes: Vec<_> = commits.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![OLD_PANE, NEW_PANE, COMMITS_PANE]);
        assert!(commits.dividers.is_empty());
        let band = commits.slots.last().expect("a commits band");
        assert_eq!(band.geometry.outer.width, narrow.width);
        assert_eq!(
            band.geometry.outer.y,
            narrow.y + narrow.height - band.geometry.outer.height
        );
        assert_eq!(
            band.geometry.outer.y + band.geometry.outer.height,
            narrow.y + narrow.height
        );

        let old = &commits.slots[0];
        let new = &commits.slots[1];
        assert_eq!(old.geometry.outer.y, narrow.y);
        assert_eq!(
            old.geometry.outer.y + old.geometry.outer.height,
            new.geometry.outer.y
        );
        assert_eq!(
            new.geometry.outer.y + new.geometry.outer.height,
            band.geometry.outer.y
        );

        let tree = body_layout(narrow, 30, &specs, Some(TREE_FOCUS), &Theme::dark());
        let panes: Vec<_> = tree.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![TREE_PANE, COMMITS_PANE]);
        assert_eq!(tree.slots[1].geometry.outer, band.geometry.outer);
    }

    #[test]
    fn commits_band_spans_the_width_across_the_responsive_boundary() {
        let specs = [TREE, OLD, NEW, COMMITS];
        for width in [80u16, 79] {
            let content = Rect::new(0, 1, width, 20);
            let layout = body_layout(content, 30, &specs, Some(DIFF_BAND_FOCUS), &Theme::dark());
            let band = layout
                .slots
                .iter()
                .find(|slot| slot.pane == COMMITS_PANE)
                .expect("a commits band");
            assert_eq!(band.geometry.outer.x, content.x);
            assert_eq!(band.geometry.outer.width, width);
            assert_eq!(
                band.geometry.outer.y + band.geometry.outer.height,
                content.y + content.height
            );
        }

        let tiny = Rect::new(0, 1, 79, 6);
        let layout = body_layout(tiny, 30, &specs, Some(DIFF_BAND_FOCUS), &Theme::dark());
        let band = layout
            .slots
            .iter()
            .find(|slot| slot.pane == COMMITS_PANE)
            .expect("a commits band");
        assert_eq!(band.geometry.outer.height, 3);
        assert_eq!(band.geometry.outer.y, tiny.y + tiny.height - 3);
        let old = layout
            .slots
            .iter()
            .find(|slot| slot.pane == OLD_PANE)
            .expect("an old side");
        let new = layout
            .slots
            .iter()
            .find(|slot| slot.pane == NEW_PANE)
            .expect("a new side");
        assert_eq!(old.geometry.outer.y, tiny.y);
        assert_eq!(
            old.geometry.outer.y + old.geometry.outer.height,
            new.geometry.outer.y
        );
        assert_eq!(
            new.geometry.outer.y + new.geometry.outer.height,
            band.geometry.outer.y
        );
        let stacked: u16 = layout
            .slots
            .iter()
            .filter(|slot| matches!(slot.pane, OLD_PANE | NEW_PANE))
            .map(|slot| slot.geometry.outer.height)
            .sum();
        assert_eq!(stacked, 3);
    }

    #[test]
    fn a_wide_layout_without_a_navigator_gives_the_bodies_the_full_width() {
        let source = body_layout(content(), 30, &[SOURCE], Some(SOURCE_FOCUS), &Theme::dark());
        assert_eq!(source.slots.len(), 1);
        assert_eq!(source.slots[0].pane, SOURCE_PANE);
        assert_eq!(source.slots[0].geometry.edge, Edge::Solo);
        assert!(source.dividers.is_empty());
        assert_eq!(source.slots[0].geometry.outer, content());

        let diff = body_layout(
            content(),
            30,
            &[OLD, NEW],
            Some(DIFF_BODY_FOCUS),
            &Theme::dark(),
        );
        let panes: Vec<PaneId> = diff.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![OLD_PANE, NEW_PANE]);
        assert_eq!(diff.slots[0].geometry.edge, Edge::Left);
        assert_eq!(diff.slots[1].geometry.edge, Edge::Right);
        assert_eq!(diff.dividers.len(), 1);
        assert_eq!(
            diff.slots[0].geometry.outer.width + 1 + diff.slots[1].geometry.outer.width,
            content().width
        );
    }

    #[test]
    fn a_band_with_one_body_pane_splits_off_the_bottom() {
        let specs = [OLD, COMMITS];
        let wide = body_layout(content(), 30, &specs, Some(DIFF_BODY_FOCUS), &Theme::dark());
        let panes: Vec<PaneId> = wide.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![OLD_PANE, COMMITS_PANE]);
        assert_eq!(wide.slots[0].geometry.edge, Edge::Solo);
        assert_eq!(wide.slots[0].geometry.outer.height, content().height - 9);
        let band = &wide.slots[1];
        assert_eq!(band.geometry.edge, Edge::Solo);
        assert_eq!(band.geometry.outer.height, 9);

        let narrow = Rect::new(0, 1, 60, 20);
        let stacked = body_layout(narrow, 30, &specs, Some(DIFF_BODY_FOCUS), &Theme::dark());
        let panes: Vec<PaneId> = stacked.slots.iter().map(|slot| slot.pane).collect();
        assert_eq!(panes, vec![OLD_PANE, COMMITS_PANE]);
        let band = &stacked.slots[1];
        assert_eq!(stacked.slots[0].geometry.outer.y, narrow.y);
        assert_eq!(
            stacked.slots[0].geometry.outer.y + stacked.slots[0].geometry.outer.height,
            band.geometry.outer.y
        );
        assert_eq!(
            band.geometry.outer.y + band.geometry.outer.height,
            narrow.y + narrow.height
        );
    }

    #[test]
    fn a_narrow_body_stack_sums_to_the_strip_height() {
        let narrow = Rect::new(0, 1, 60, 20);
        let specs = [TREE, OLD, NEW, COMMITS];
        let layout = body_layout(narrow, 30, &specs, Some(DIFF_BODY_FOCUS), &Theme::dark());
        let band = layout
            .slots
            .iter()
            .find(|slot| slot.pane == COMMITS_PANE)
            .expect("a commits band");
        let stacked: u16 = layout
            .slots
            .iter()
            .filter(|slot| matches!(slot.pane, OLD_PANE | NEW_PANE))
            .map(|slot| slot.geometry.outer.height)
            .sum();
        assert_eq!(stacked, band.geometry.outer.y - narrow.y);
    }

    #[test]
    fn geometry_inner_is_the_pane_block_inner_rect() {
        let theme = Theme::dark();
        let wide = body_layout(
            content(),
            30,
            &[TREE, OLD, NEW],
            Some(DIFF_BODY_FOCUS),
            &theme,
        );
        assert!(!wide.slots.is_empty());
        for slot in &wide.slots {
            let expected =
                pane_block("", false, &theme, slot.geometry.edge).inner(slot.geometry.outer);
            assert_eq!(slot.geometry.inner, expected);
        }

        let narrow = Rect::new(0, 1, 60, 20);
        let stacked = body_layout(narrow, 30, &[TREE, OLD, NEW], Some(DIFF_BODY_FOCUS), &theme);
        assert!(!stacked.slots.is_empty());
        for slot in &stacked.slots {
            let expected =
                pane_block("", false, &theme, slot.geometry.edge).inner(slot.geometry.outer);
            assert_eq!(slot.geometry.inner, expected);
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
