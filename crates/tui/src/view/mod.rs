// The pure frame layer.
//
// `view` composes a header, a body, and a footer, then draws overlays and
// diagnostics on top. The body is laid out here, but each pane draws itself:
// the tree and commits picker from [`crate::component`], and the show and diff
// bodies from their page. Every module here is presentation-only: it reads the
// model and writes to the frame, and performs no I/O.
//
// The whole canvas is painted with the palette background before any pane is
// drawn, so a terminal whose background differs from the palette never shows
// through the panes, gutters, or the too-small notice.

pub(crate) mod chrome;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Block;

use crate::app::App;
use crate::component::{RenderCtx, commit_picker, overlay, tree};
use crate::page::diff::Side;
use crate::page::{DiffViewState, Loaded, ViewCtx, diff as diff_page, show as show_page};
use crate::plugin::{COMMITS_PANE, NEW_PANE, OLD_PANE, SOURCE_PANE, TREE_PANE};
use crate::render::block::render_divider;
use crate::render::layout::{body_layout, frame_areas};
use crate::render::metrics::{MIN_HEIGHT, SINGLE_PANE_MIN_WIDTH};

pub(crate) fn view(app: &App, frame: &mut Frame) {
    let area = frame.area();

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

fn render_body(app: &App, frame: &mut Frame, content: Rect) {
    let panes = app.loaded.panes();
    let focus = app.focus;
    let layout = body_layout(
        content,
        app.chrome.tree_percent,
        &panes,
        focus,
        &app.chrome.theme,
    );
    let ctx = RenderCtx {
        theme: &app.chrome.theme,
        icons: &app.chrome.icons,
        workspace: &app.workspace,
        busy: app.is_busy(),
    };
    let vctx = ViewCtx {
        theme: &app.chrome.theme,
        busy: app.is_busy(),
        tree_empty: app.workspace.files.paths.is_empty(),
    };

    for slot in &layout.slots {
        let focused = focus == Some(slot.focus);
        let outer = slot.geometry.outer;
        let edge = slot.geometry.edge;
        if slot.pane == TREE_PANE {
            tree::render(&app.tree, &ctx, frame, outer, focused, edge);
        } else if slot.pane == SOURCE_PANE {
            if let Loaded::Show(show) = &app.loaded {
                show_page::view(show, &app.workspace, &vctx, frame, outer, focused, edge);
            }
        } else if slot.pane == OLD_PANE {
            if let Loaded::Diff(diff) = &app.loaded {
                diff_page::view(
                    diff,
                    &app.workspace,
                    &vctx,
                    frame,
                    outer,
                    Side::Old,
                    focused,
                    edge,
                );
            }
        } else if slot.pane == NEW_PANE {
            if let Loaded::Diff(diff) = &app.loaded {
                diff_page::view(
                    diff,
                    &app.workspace,
                    &vctx,
                    frame,
                    outer,
                    Side::New,
                    focused,
                    edge,
                );
            }
        } else if slot.pane == COMMITS_PANE
            && let Loaded::Diff(diff) = &app.loaded
            && let DiffViewState::Commits(commits) = &diff.view
        {
            commit_picker::render(&commits.picker, &ctx, frame, outer, focused, edge);
        }
    }

    for divider in &layout.dividers {
        render_divider(frame, *divider, &app.chrome.theme);
    }
}

fn render_overlay(app: &App, frame: &mut Frame, area: Rect) {
    match &app.overlay {
        Some(state) => {
            let entries = app.loaded.palette_entries();
            let groups = app.loaded.prefix_groups();
            let ctx = overlay::RenderCtx {
                theme: &app.chrome.theme,
                mode: app.workspace.query.mode,
                entries: &entries,
                groups: &groups,
                visible: &app.workspace.files.paths,
                matches: app
                    .workspace
                    .search
                    .as_ref()
                    .map_or(0, |search| search.matches.len()),
                pending_prefix: app.keymap.pending,
            };
            overlay::render(state, &ctx, frame, area);
        }
        None => render_prompt(app, frame, area),
    }
}

fn render_prompt(app: &App, frame: &mut Frame, area: Rect) {
    match &app.loaded {
        Loaded::Show(show) => show_page::prompt_view(show, &app.chrome.theme, frame, area),
        Loaded::Diff(diff) => diff_page::prompt_view(diff, &app.chrome.theme, frame, area),
    }
}
