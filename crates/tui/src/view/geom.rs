// Pure geometry and panel chrome shared by the view and the derived-layout
// cache. Nothing here reads the terminal or the model's behaviour.

use base::FileDiff;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};

use crate::app::{Focus, SIDE_BY_SIDE_MIN_WIDTH};
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

fn borders_for(edge: Edge) -> Borders {
    match edge {
        Edge::Solo => Borders::ALL,
        Edge::Left => Borders::LEFT | Borders::TOP | Borders::BOTTOM,
        Edge::Middle => Borders::TOP | Borders::BOTTOM,
        Edge::Right => Borders::RIGHT | Borders::TOP | Borders::BOTTOM,
    }
}

pub(crate) fn pane_block(title: &str, focused: bool, theme: &Theme, edge: Edge) -> Block<'static> {
    let border = if focused {
        theme.fg(theme.palette.border_focus)
    } else {
        theme.fg(theme.palette.border)
    };
    Block::default()
        .borders(borders_for(edge))
        .border_type(BorderType::Rounded)
        .border_style(border)
        .padding(Padding::horizontal(1))
        .style(theme.bg(theme.palette.bg))
        .title(title.to_owned())
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

pub(crate) fn render_divider(frame: &mut Frame, column: Rect, theme: &Theme) {
    if column.width == 0 || column.height == 0 {
        return;
    }
    let style = theme.fg(theme.palette.divider);
    let mut lines = Vec::with_capacity(column.height as usize);
    for row in 0..column.height {
        let symbol = match row {
            0 => "┬",
            row if row + 1 == column.height => "┴",
            _ => "│",
        };
        lines.push(ratatui::text::Line::from(Span::styled(
            symbol.to_owned(),
            style,
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(theme.bg(theme.palette.bg)),
        column,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
