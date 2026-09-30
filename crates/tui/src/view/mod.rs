// The pure view layer.
//
// `view` composes a header, a body, and a footer, then draws overlays and
// diagnostics on top. Every module here is presentation-only: it reads the
// model and writes to the frame, and performs no I/O. Layout arithmetic lives
// in [`crate::layout`] and text helpers in [`crate::text`]; the isolated panes
// render themselves from [`crate::components`].

pub(crate) mod block;
pub(crate) mod chrome;
pub(crate) mod diff;
pub(crate) mod empty;
pub(crate) mod prompt;
pub(crate) mod show;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Block;

use crate::app::{App, Focus, MIN_HEIGHT, SINGLE_PANE_MIN_WIDTH, layout_focus};
use crate::components::{RenderCtx, commit_picker, overlay, tree};
use crate::content::{CommitsFocus, DiffViewState, Loaded};
use crate::layout::{PaneSlot, body_layout, frame_areas};

use block::render_divider;
use diff::Side;

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
        render_overlay(app, frame, area);
        return;
    }

    let (header, content, status) = frame_areas(area.width, area.height);

    chrome::render_header(app, frame, header);
    render_body(app, frame, content);
    chrome::render_status(app, frame, status);
    chrome::render_diagnostic(app, frame, area);
    render_overlay(app, frame, area);
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
    let ctx = RenderCtx {
        theme: &app.chrome.theme,
        icons: &app.chrome.icons,
        loaded: &app.loaded,
        busy: app.is_busy(),
    };

    for slot in &layout.slots {
        match slot.pane {
            PaneSlot::Commits => {
                if let Loaded::Diff(diff) = &app.loaded
                    && let DiffViewState::Commits(commits) = &diff.view
                {
                    let focused = commits.focus == CommitsFocus::Commits;
                    commit_picker::render(
                        &commits.picker,
                        &ctx,
                        frame,
                        slot.outer,
                        focused,
                        slot.edge,
                    );
                }
            }
            PaneSlot::Tree => tree::render(
                &app.tree,
                &ctx,
                frame,
                slot.outer,
                focus == Focus::Tree,
                slot.edge,
            ),
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

// Draws the active overlay, or the loaded projection's revision prompt when no
// overlay is open. The overlay state owns its keys; the prompt belongs to the
// content, so the two are drawn from different places.
fn render_overlay(app: &App, frame: &mut Frame, area: Rect) {
    match &app.overlay {
        Some(state) => {
            let entries = app.loaded.palette_entries();
            let ctx = overlay::RenderCtx {
                theme: &app.chrome.theme,
                mode: app.loaded.mode(),
                entries: &entries,
                visible: app.loaded.visible(),
                matches: app.loaded.search().map_or(0, |search| search.matches.len()),
            };
            overlay::render(state, &ctx, frame, area);
        }
        None => prompt::render(app, frame, area),
    }
}
