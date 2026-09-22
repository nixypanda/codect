//! The pure view layer.
//!
//! `view` composes a header, a body, and a footer, then draws overlays and
//! diagnostics on top. Every module here is presentation-only: it reads the
//! model and writes to the frame, and performs no I/O.

pub(crate) mod chrome;
pub(crate) mod diff;
pub(crate) mod geom;
pub(crate) mod overlay;
pub(crate) mod show;
pub(crate) mod text;
pub(crate) mod tree;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Block;

use crate::app::{Content, MIN_HEIGHT, Model, Pane, SINGLE_PANE_MIN_WIDTH};

use diff::Side;
use geom::{PaneSlot, body_layout, frame_areas, render_divider};

/// Renders the whole model. Pure: it reads the model and writes to the frame.
pub(crate) fn view(model: &Model, frame: &mut Frame) {
    let area = frame.area();

    // The palette owns every cell: paint the whole canvas before drawing so a
    // terminal whose background differs from the palette never shows through
    // the panes, gutters, or the too-small notice.
    frame.render_widget(
        Block::default().style(model.theme.bg(model.theme.palette.bg)),
        area,
    );

    if area.width < SINGLE_PANE_MIN_WIDTH || area.height < MIN_HEIGHT {
        chrome::render_too_small(frame, area, &model.theme);
        overlay::render_overlay(model, frame, area);
        return;
    }

    let (header, content, status) = frame_areas(area.width, area.height);

    chrome::render_header(model, frame, header);
    render_body(model, frame, content);
    chrome::render_status(model, frame, status);
    chrome::render_diagnostic(model, frame, area);
    overlay::render_overlay(model, frame, area);
}

/// Renders the body panes from the same layout mouse hit-testing uses.
fn render_body(model: &Model, frame: &mut Frame, content: Rect) {
    let is_diff = matches!(model.content, Content::Diff(_));
    let layout = body_layout(content, model.tree_percent, is_diff, model.focus);
    let rows = model.diff_rows();
    let diff_focused = model.focus == Pane::Diff;

    for slot in &layout.slots {
        match slot.pane {
            PaneSlot::Tree => tree::render_tree(
                model,
                frame,
                slot.outer,
                model.focus == Pane::Tree,
                slot.edge,
            ),
            PaneSlot::Show => show::render_show_body(
                model,
                frame,
                slot.outer,
                model.focus == Pane::Body,
                slot.edge,
            ),
            PaneSlot::Old => diff::render_diff_pane(
                model,
                frame,
                slot.outer,
                Side::Old,
                diff_focused,
                rows,
                slot.edge,
            ),
            PaneSlot::New => diff::render_diff_pane(
                model,
                frame,
                slot.outer,
                Side::New,
                diff_focused,
                rows,
                slot.edge,
            ),
        }
    }

    for divider in &layout.dividers {
        render_divider(frame, *divider, &model.theme);
    }
}
