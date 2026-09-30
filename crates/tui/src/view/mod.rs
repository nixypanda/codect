// The pure view layer.
//
// `view` composes a header, a body, and a footer, then draws overlays and
// diagnostics on top. Every module here is presentation-only: it reads the
// model and writes to the frame, and performs no I/O.

pub(crate) mod chrome;
pub(crate) mod commits;
pub(crate) mod diff;
mod empty;
pub(crate) mod geom;
pub(crate) mod overlay;
pub(crate) mod show;
pub(crate) mod text;
pub(crate) mod tree;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Block;

use crate::app::{App, Focus, MIN_HEIGHT, SINGLE_PANE_MIN_WIDTH, layout_focus};
use crate::content::{DiffViewState, Loaded};

use diff::Side;
use geom::{PaneSlot, body_layout, frame_areas, render_divider};

pub(crate) fn view(app: &App, frame: &mut Frame) {
    let area = frame.area();

    // The palette owns every cell: paint the whole canvas before drawing so a
    // terminal whose background differs from the palette never shows through
    // the panes, gutters, or the too-small notice.
    frame.render_widget(
        Block::default().style(app.chrome.theme.bg(app.chrome.theme.palette.bg)),
        area,
    );

    if area.width < SINGLE_PANE_MIN_WIDTH || area.height < MIN_HEIGHT {
        chrome::render_too_small(frame, area, &app.chrome.theme);
        overlay::render_overlay(app, frame, area);
        return;
    }

    let (header, content, status) = frame_areas(area.width, area.height);

    chrome::render_header(app, frame, header);
    render_body(app, frame, content);
    chrome::render_status(app, frame, status);
    chrome::render_diagnostic(app, frame, area);
    overlay::render_overlay(app, frame, area);
}

// Renders the body panes from the same layout mouse hit-testing uses.
fn render_body(app: &App, frame: &mut Frame, content: Rect) {
    let is_diff = matches!(app.loaded, Loaded::Diff(_));
    let has_commits = matches!(
        &app.loaded,
        Loaded::Diff(diff) if matches!(diff.view, DiffViewState::Commits(_))
    );
    let focus = layout_focus(app);
    let layout = body_layout(
        content,
        app.chrome.tree_percent,
        is_diff,
        has_commits,
        focus,
    );
    let rows = app.diff_rows();
    let diff_focused = matches!(&app.loaded, Loaded::Diff(diff) if diff.focus_is_diff());

    for slot in &layout.slots {
        match slot.pane {
            PaneSlot::Commits => commits::render_commits(app, frame, slot.outer, slot.edge),
            PaneSlot::Tree => {
                tree::render_tree(app, frame, slot.outer, focus == Focus::Tree, slot.edge)
            }
            PaneSlot::Show => {
                show::render_show_body(app, frame, slot.outer, focus == Focus::Content, slot.edge)
            }
            PaneSlot::Old => diff::render_diff_pane(
                app,
                frame,
                slot.outer,
                Side::Old,
                diff_focused,
                rows,
                slot.edge,
            ),
            PaneSlot::New => diff::render_diff_pane(
                app,
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
        render_divider(frame, *divider, &app.chrome.theme);
    }
}
