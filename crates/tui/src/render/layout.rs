//! Layout arithmetic shared by the view, the derived-layout cache, and mouse
//! hit-testing: where the panes sit and the offsets that keep a cursor visible.
//!
//! Everything here is pure. It reads no terminal and no model behaviour; the
//! values it needs are passed in, including the focus and the responsive width
//! constants it shares with the shell from [`crate::render::metrics`].
//!
//! The layout tiles its area exactly: adjacent panes share one divider rather
//! than doubling it, and the commits band is pinned to the bottom, capped, and
//! keeps a minimum strip above it. `body_layout` is the single source of truth
//! shared with mouse hit-testing, so a click or wheel maps to the pane that was
//! drawn.

use ratatui::layout::{Constraint, Layout, Rect};

use crate::plugin::{FocusKey, PaneId};
use crate::render::block::pane_block;
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

fn edge_for(index: usize, last: usize) -> Edge {
    match (index, last) {
        (0, 0) => Edge::Solo,
        (0, _) => Edge::Left,
        (index, last) if index == last => Edge::Right,
        _ => Edge::Middle,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
