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
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{
    Content, MIN_HEIGHT, Model, Pane, SIDE_BY_SIDE_MIN_WIDTH, SINGLE_PANE_MIN_WIDTH,
};

use diff::Side;
use geom::{Edge, render_divider, split_with_dividers};

/// Renders the whole model. Pure: it reads the model and writes to the frame.
pub(crate) fn view(model: &Model, frame: &mut Frame) {
    let area = frame.area();

    if area.width < SINGLE_PANE_MIN_WIDTH || area.height < MIN_HEIGHT {
        chrome::render_too_small(frame, area, &model.theme);
        overlay::render_overlay(model, frame, area);
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);
    let (header, content, status) = (chunks[0], chunks[1], chunks[2]);

    chrome::render_header(model, frame, header);
    render_body(model, frame, content);
    chrome::render_status(model, frame, status);
    chrome::render_diagnostic(model, frame, area);
    overlay::render_overlay(model, frame, area);
}

fn render_body(model: &Model, frame: &mut Frame, content: Rect) {
    let side_by_side = frame.area().width >= SIDE_BY_SIDE_MIN_WIDTH;
    match &model.content {
        Content::Show(_) => {
            if side_by_side {
                let (columns, dividers) = split_with_dividers(
                    content,
                    &[model.tree_percent, 100 - model.tree_percent],
                );
                tree::render_tree(model, frame, columns[0], model.focus == Pane::Tree, Edge::Left);
                render_divider(frame, dividers[0], &model.theme);
                show::render_show_body(model, frame, columns[1], model.focus == Pane::Body, Edge::Right);
            } else if model.focus == Pane::Tree {
                tree::render_tree(model, frame, content, true, Edge::Solo);
            } else {
                show::render_show_body(model, frame, content, true, Edge::Solo);
            }
        }
        Content::Diff(_) => {
            let rows = model.diff_rows();
            let diff_focused = model.focus == Pane::Diff;
            if side_by_side {
                let rest = 100 - model.tree_percent;
                let side = rest / 2;
                let (columns, dividers) =
                    split_with_dividers(content, &[model.tree_percent, side, rest - side]);
                tree::render_tree(model, frame, columns[0], model.focus == Pane::Tree, Edge::Left);
                render_divider(frame, dividers[0], &model.theme);
                diff::render_diff_pane(
                    model,
                    frame,
                    columns[1],
                    Side::Old,
                    diff_focused,
                    rows,
                    Edge::Middle,
                );
                render_divider(frame, dividers[1], &model.theme);
                diff::render_diff_pane(
                    model,
                    frame,
                    columns[2],
                    Side::New,
                    diff_focused,
                    rows,
                    Edge::Right,
                );
            } else {
                match model.focus {
                    Pane::Tree => tree::render_tree(model, frame, content, true, Edge::Solo),
                    Pane::Diff => {
                        // Too narrow for side by side: stack old over new; both
                        // halves stay synchronized on the same aligned rows.
                        let halves = Layout::vertical([
                            Constraint::Percentage(50),
                            Constraint::Percentage(50),
                        ])
                        .split(content);
                        diff::render_diff_pane(
                            model,
                            frame,
                            halves[0],
                            Side::Old,
                            true,
                            rows,
                            Edge::Solo,
                        );
                        diff::render_diff_pane(
                            model,
                            frame,
                            halves[1],
                            Side::New,
                            true,
                            rows,
                            Edge::Solo,
                        );
                    }
                    Pane::Body => {}
                }
            }
        }
    }
}
