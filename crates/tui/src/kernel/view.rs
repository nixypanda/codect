//! Frame composition over the layout and the overlay slot.
//!
//! The kernel draws the frame; for each visible slot it calls the owning
//! plugin's `render`, then draws the dividers and the overlay.

#![allow(dead_code)]
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Block;

use crate::kernel::app::{App, Specs, cx_for};
use crate::kernel::chrome;
use crate::kernel::layout::{MIN_HEIGHT, SINGLE_PANE_MIN_WIDTH, frame_areas};
use crate::kernel::plugin::{HintTarget, PluginSet};
use crate::render::block::render_divider;

pub(crate) fn view<S: PluginSet>(app: &App<S>, frame: &mut Frame) {
    // The body layout needs only the panes. The command registry and help are
    // built only when an overlay may read them, so an idle warm frame does not
    // reallocate the whole registry.
    let overlay = app.plugins.overlay_pane();
    let specs = if overlay.is_some() {
        Specs::full(app)
    } else {
        Specs::panes_only(app)
    };
    let busy = app.plugins.busy();
    let mut cx = cx_for(&app.kernel, overlay, busy, &specs);
    let area = frame.area();
    cx.size = crate::kernel::plugin::Size {
        width: area.width,
        height: area.height,
    };
    let theme = cx.theme;

    frame.render_widget(Block::default().style(theme.bg(theme.palette.bg)), area);

    if area.width < SINGLE_PANE_MIN_WIDTH || area.height < MIN_HEIGHT {
        chrome::render_too_small(frame, area, theme);
        render_overlay(app, &cx, frame, area);
        return;
    }

    let (header, content, status) = frame_areas(area.width, area.height);

    let chips = app.plugins.chips(&cx);
    chrome::render_header(
        cx.root,
        cx.workspace.query.mode,
        &chips,
        &cx.workspace.query.scope.label,
        theme,
        frame,
        header,
    );

    // Reuse the content rect the chrome solve already produced: a second
    // `frame_areas` call would repeat the whole layout solve every draw.
    let layout = cx.layout_in(content);
    for slot in &layout.slots {
        let focused = cx.focus == Some(slot.focus);
        app.plugins
            .render(slot.pane, slot.geometry, focused, &cx, frame);
    }
    for divider in &layout.dividers {
        render_divider(frame, *divider, theme);
    }

    let segments = app.plugins.status(&cx);
    let hints = match cx.overlay {
        Some(pane) => app.plugins.hints(HintTarget::Overlay(pane), &cx),
        None => cx
            .focus
            .and_then(|focus| app.plugins.hints(HintTarget::Focus(focus), &cx)),
    };
    let compact = chrome::compact_hint(cx.overlay.map(|pane| app.plugins.overlay_spec(pane)));
    let selected = cx.workspace.selected.as_ref().map(ToString::to_string);
    chrome::render_status(
        busy,
        app.kernel.chrome.spinner,
        selected.as_deref(),
        &segments,
        cx.workspace.search.as_ref(),
        hints.unwrap_or(""),
        compact,
        theme,
        frame,
        status,
    );

    chrome::render_diagnostic(
        app.kernel.diagnostic.as_ref().map(|d| d.text.as_str()),
        theme,
        frame,
        area,
    );

    render_overlay(app, &cx, frame, area);
}

fn render_overlay<S: PluginSet>(
    app: &App<S>,
    cx: &crate::kernel::cx::Cx,
    frame: &mut Frame,
    area: Rect,
) {
    if let Some(pane) = cx.overlay {
        app.plugins.render_overlay(pane, area, cx, frame);
    }
}
