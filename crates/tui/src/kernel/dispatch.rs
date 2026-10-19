//! Dispatch: the depth-first request loop, key dispatch, and mouse dispatch.

#![allow(dead_code)]
use crate::kernel::app::{App, Diagnostic, Specs, cx_for};
use crate::kernel::command::{CommandId, Owner};
use crate::kernel::focus::{first_pane_with_focus, focus_default};
use crate::kernel::keymap;
use crate::kernel::layout::{MIN_HEIGHT, SINGLE_PANE_MIN_WIDTH, body_layout, frame_areas};
use crate::kernel::nav::Nav;
use crate::kernel::plugin::{
    Event, EventTarget, Input, MouseInput, Out, PaneId, PluginId, PluginSet, Reply, Request,
};
use crate::kernel::search::{Search, advance_search, current_match};
use crate::util::input::{Key, Mouse};

/// The depth a request cascade may reach before the kernel gives up.
const REQUEST_DEPTH_LIMIT: u8 = 16;

/// Apply `outs` whose requests nest at `depth`.
pub(crate) fn process_wrapped<S: PluginSet>(
    app: &mut App<S>,
    outs: Vec<(PluginId, Out<S::Cmd>)>,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    process_outs(
        app,
        outs.into_iter().map(|(_, out)| out),
        cmds,
        depth,
        specs,
    );
}

/// Apply `outs`, preserving interleaved order: a request is applied where it
/// appeared relative to its sibling effects.
pub(crate) fn process_outs<S: PluginSet>(
    app: &mut App<S>,
    outs: impl IntoIterator<Item = Out<S::Cmd>>,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    for out in outs {
        match out {
            Out::Effect(cmd) => cmds.push(cmd),
            Out::Kernel(request) => apply_request(app, request, cmds, depth, specs),
        }
    }
}

fn deliver<S: PluginSet>(
    app: &mut App<S>,
    target: EventTarget,
    event: &Event,
    specs: &Specs,
) -> Vec<(PluginId, Out<S::Cmd>)> {
    let App { kernel, plugins } = app;
    let overlay = kernel.overlay;
    let busy = plugins.busy();
    let cx = cx_for(kernel, overlay, busy, specs);
    plugins.on_event(target, event, &cx)
}

/// Broadcast `event` to every plugin and process its outs.
pub(crate) fn broadcast<S: PluginSet>(
    app: &mut App<S>,
    event: Event,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    let outs = deliver(app, EventTarget::All, &event, specs);
    process_wrapped(app, outs, cmds, depth + 1, specs);
}

/// Deliver `event` to one plugin's owner and process its outs.
fn send_to<S: PluginSet>(
    app: &mut App<S>,
    owner: PluginId,
    event: Event,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    let outs = deliver(app, EventTarget::One(owner), &event, specs);
    process_wrapped(app, outs, cmds, depth + 1, specs);
}

fn plugin_input<S: PluginSet>(
    app: &mut App<S>,
    pane: PaneId,
    input: Input,
    specs: &Specs,
) -> Reply<S::Cmd> {
    let App { kernel, plugins } = app;
    let overlay = kernel.overlay;
    let busy = plugins.busy();
    let cx = cx_for(kernel, overlay, busy, specs);
    plugins.input(pane, input, &cx)
}

/// Apply a kernel request and any requests its handlers produce.
pub(crate) fn apply_request<S: PluginSet>(
    app: &mut App<S>,
    request: Request,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    if depth >= REQUEST_DEPTH_LIMIT {
        debug_assert!(false, "request cascade exceeded depth");
        app.kernel.diagnostic = Some(Diagnostic::new("request depth exceeded".to_owned()));
        return;
    }
    apply_one(app, request, cmds, depth, specs);
}

#[allow(clippy::too_many_lines)]
fn apply_one<S: PluginSet>(
    app: &mut App<S>,
    request: Request,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    match request {
        Request::Quit => app.kernel.chrome.quit = true,
        Request::Diagnose(text) => app.kernel.diagnostic = Some(Diagnostic::new(text)),
        Request::ClearDiagnostic => app.kernel.diagnostic = None,
        Request::SetFiles(set) => {
            if let Some(selected) = app.kernel.workspace.selected.as_ref()
                && !set.paths.iter().any(|path| path == selected)
            {
                app.kernel.workspace.selected = None;
            }
            app.kernel.workspace.files = set;
            broadcast(app, Event::FilesChanged, cmds, depth, specs);
        }
        Request::Select(selection) => {
            let had_prefix = app.kernel.keymap.pending.is_some();
            app.kernel.keymap.pending = None;
            if had_prefix {
                broadcast(app, Event::PrefixCleared, cmds, depth, specs);
            }
            app.kernel.workspace.selected = selection;
            broadcast(app, Event::SelectionChanged, cmds, depth, specs);
        }
        Request::Requery(change) => {
            broadcast(app, Event::Requery(change), cmds, depth, specs);
        }
        Request::QueryLoaded(query) => {
            app.kernel.workspace.query = query;
            broadcast(app, Event::QueryChanged, cmds, depth, specs);
        }
        Request::SetSearch(needle) => {
            if needle.is_empty() {
                app.kernel.workspace.search = None;
            } else {
                let matches = search_needle(app, &needle, specs);
                app.kernel.workspace.search = Some(Search {
                    needle,
                    matches,
                    cursor: 0,
                });
            }
            broadcast(app, Event::SearchChanged, cmds, depth, specs);
        }
        Request::CommitSearch => {
            if let Some(matched) = current_match(&app.kernel.workspace.search) {
                broadcast(app, Event::RevealMatch(matched), cmds, depth, specs);
            }
        }
        Request::ClearSearch => {
            app.kernel.workspace.search = None;
            broadcast(app, Event::SearchChanged, cmds, depth, specs);
        }
        Request::Focus(focus) => app.kernel.focus = Some(focus),
        Request::FocusDefault => app.kernel.focus = focus_default(&specs.panes),
        Request::CycleFocus { forward } => cycle_focus(app, forward, specs),
        Request::ResizeNavigator(resize) => {
            app.kernel.chrome.tree_percent = match resize {
                crate::kernel::plugin::NavigatorResize::Wider => {
                    (app.kernel.chrome.tree_percent + 5).min(60)
                }
                crate::kernel::plugin::NavigatorResize::Narrower => {
                    app.kernel.chrome.tree_percent.saturating_sub(5).max(15)
                }
                crate::kernel::plugin::NavigatorResize::Reset => 30,
            };
        }
        Request::OpenOverlay(pane) => {
            let previous = app.kernel.overlay;
            app.kernel.overlay = Some(pane);
            if let Some(previous) = previous
                && previous != pane
            {
                let owner = app.plugins.pane_owner(previous).unwrap_or(previous.owner);
                send_to(
                    app,
                    owner,
                    Event::OverlayClosed(previous),
                    cmds,
                    depth,
                    specs,
                );
            }
        }
        Request::CloseOverlay(pane) => {
            if app.kernel.overlay == Some(pane) {
                app.kernel.overlay = None;
            }
        }
        Request::CancelPrefix => {
            if app.kernel.keymap.pending.take().is_some() {
                broadcast(app, Event::PrefixCleared, cmds, depth, specs);
            }
        }
        Request::RunCommand(id) => run_command(app, id, cmds, depth, specs),
    }
}

fn search_needle<S: PluginSet>(
    app: &mut App<S>,
    needle: &str,
    specs: &Specs,
) -> Vec<crate::kernel::search::SearchMatch> {
    let App { kernel, plugins } = app;
    let overlay = kernel.overlay;
    let busy = plugins.busy();
    let cx = cx_for(kernel, overlay, busy, specs);
    plugins.search(needle, &cx)
}

fn cycle_focus<S: PluginSet>(app: &mut App<S>, forward: bool, specs: &Specs) {
    let ring = crate::kernel::focus::focus_ring(&specs.panes);
    if ring.is_empty() {
        return;
    }
    let index = app
        .kernel
        .focus
        .and_then(|focus| ring.iter().position(|key| *key == focus))
        .unwrap_or(0);
    let next = if forward {
        (index + 1) % ring.len()
    } else {
        (index + ring.len() - 1) % ring.len()
    };
    app.kernel.focus = Some(ring[next]);
}

fn run_command<S: PluginSet>(
    app: &mut App<S>,
    id: CommandId,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    if id.owner == Owner::Kernel {
        run_kernel_command(app, id.name, cmds, depth, specs);
        return;
    }
    let outs = {
        let App { kernel, plugins } = app;
        let overlay = kernel.overlay;
        let busy = plugins.busy();
        let cx = cx_for(kernel, overlay, busy, specs);
        plugins.run(id, &cx)
    };
    process_wrapped(app, outs, cmds, depth + 1, specs);
}

fn run_kernel_command<S: PluginSet>(
    app: &mut App<S>,
    name: &str,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    match name {
        "quit" => apply_request(app, Request::Quit, cmds, depth, specs),
        "pane.next" => apply_request(
            app,
            Request::CycleFocus { forward: true },
            cmds,
            depth,
            specs,
        ),
        "pane.previous" => {
            apply_request(
                app,
                Request::CycleFocus { forward: false },
                cmds,
                depth,
                specs,
            );
        }
        "nav.top" => send_nav(app, Nav::Top, cmds, depth, specs),
        "nav.bottom" => send_nav(app, Nav::Bottom, cmds, depth, specs),
        "nav.page_up" => send_nav(app, Nav::PageUp, cmds, depth, specs),
        "nav.page_down" => send_nav(app, Nav::PageDown, cmds, depth, specs),
        "search.next" => reveal_next(app, true, cmds, depth, specs),
        "search.previous" => reveal_next(app, false, cmds, depth, specs),
        _ => {}
    }
}

fn send_nav<S: PluginSet>(
    app: &mut App<S>,
    nav: Nav,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    let Some(focus) = app.kernel.focus else {
        return;
    };
    let Some(pane) = first_pane_with_focus(&specs.panes, focus) else {
        return;
    };
    let reply = plugin_input(app, pane, Input::Nav(nav), specs);
    process_outs(app, reply.outs, cmds, depth + 1, specs);
}

fn reveal_next<S: PluginSet>(
    app: &mut App<S>,
    forward: bool,
    cmds: &mut Vec<S::Cmd>,
    depth: u8,
    specs: &Specs,
) {
    if let Some(matched) = advance_search(&mut app.kernel.workspace.search, forward) {
        broadcast(app, Event::RevealMatch(matched), cmds, depth, specs);
    }
}

/// Key dispatch, exactly the section 4.8 order.
pub(crate) fn dispatch_key<S: PluginSet>(
    app: &mut App<S>,
    key: Key,
    cmds: &mut Vec<S::Cmd>,
    specs: &Specs,
) {
    // 1. Ctrl-C always quits.
    if key == Key::CtrlC {
        apply_request(app, Request::Quit, cmds, 0, specs);
        return;
    }
    // 2. Ctrl-K/Ctrl-J alias, except inside a keeps-ctrl-jk overlay.
    let keeps = app
        .kernel
        .overlay
        .is_some_and(|pane| app.plugins.overlay_spec(pane).keeps_ctrl_jk);
    let key = if keeps {
        key
    } else {
        match key {
            Key::CtrlK => Key::Char('k'),
            Key::CtrlJ => Key::Char('j'),
            other => other,
        }
    };
    // 3. An open overlay owns the key.
    if let Some(pane) = app.kernel.overlay {
        let spec = app.plugins.overlay_spec(pane);
        if key == Key::Char('q') && !spec.captures_text {
            apply_request(app, Request::Quit, cmds, 0, specs);
            return;
        }
        let reply = plugin_input(app, pane, Input::Key(key), specs);
        let handled = reply.handled;
        process_outs(app, reply.outs, cmds, 1, specs);
        if handled || spec.modal {
            return;
        }
    }
    // 4. A pending prefix resolves or cancels.
    if let Some(prefix) = app.kernel.keymap.pending {
        apply_request(app, Request::CancelPrefix, cmds, 0, specs);
        if let Key::Char(character) = key
            && let Some(id) =
                keymap::prefixed_command(&specs.commands, prefix, character, app.kernel.focus)
        {
            apply_request(app, Request::RunCommand(id), cmds, 0, specs);
        }
        return;
    }
    // 5. `q` quits.
    if key == Key::Char('q') {
        apply_request(app, Request::Quit, cmds, 0, specs);
        return;
    }
    // 6. A prefix key arms the band.
    if let Key::Char(character) = key
        && keymap::has_prefix(&specs.commands, character, app.kernel.focus)
    {
        app.kernel.keymap.pending = Some(character);
        broadcast(app, Event::PrefixPending(character), cmds, 0, specs);
        return;
    }
    // 7. A bound key runs its command.
    if let Some(id) = keymap::command_for_key(&specs.commands, key, app.kernel.focus) {
        apply_request(app, Request::RunCommand(id), cmds, 0, specs);
        return;
    }
    // 8. Forward to the focused pane.
    if let Some(focus) = app.kernel.focus
        && let Some(pane) = first_pane_with_focus(&specs.panes, focus)
    {
        let reply = plugin_input(app, pane, Input::Key(key), specs);
        let handled = reply.handled;
        process_outs(app, reply.outs, cmds, 1, specs);
        if handled {
            return;
        }
    }
    // 9. Esc clears the diagnostic and the search.
    if key == Key::Esc {
        apply_request(app, Request::ClearDiagnostic, cmds, 0, specs);
        apply_request(app, Request::ClearSearch, cmds, 0, specs);
    }
}

/// Mouse dispatch, exactly the section 4.9 order.
pub(crate) fn dispatch_mouse<S: PluginSet>(
    app: &mut App<S>,
    event: Mouse,
    cmds: &mut Vec<S::Cmd>,
    specs: &Specs,
) {
    // 1. Cancel a pending prefix (which also closes the band).
    apply_request(app, Request::CancelPrefix, cmds, 0, specs);
    // 2. Ignore the mouse while an overlay is open or the terminal is small.
    if app.kernel.overlay.is_some()
        || app.kernel.size.width < SINGLE_PANE_MIN_WIDTH
        || app.kernel.size.height < MIN_HEIGHT
    {
        return;
    }
    let Some(focus) = app.kernel.focus.or_else(|| focus_default(&specs.panes)) else {
        return;
    };
    // 3. Find the slot under the pointer.
    let hit = {
        let (_, content, _) = frame_areas(app.kernel.size.width, app.kernel.size.height);
        let layout = body_layout(
            content,
            app.kernel.chrome.tree_percent,
            &specs.panes,
            Some(focus),
            &app.kernel.chrome.theme,
        );
        let point = ratatui::layout::Position {
            x: event.column,
            y: event.row,
        };
        layout
            .slots
            .iter()
            .find(|slot| slot.geometry.outer.contains(point))
            .map(|slot| (slot.pane, slot.focus, slot.geometry))
    };
    let Some((pane, focus, geometry)) = hit else {
        return;
    };
    // 4. Focus the hit pane.
    app.kernel.focus = Some(focus);
    // 5. Forward the event with its geometry.
    let input = Input::Mouse(MouseInput {
        kind: event.kind,
        column: event.column,
        row: event.row,
        geometry,
    });
    let reply = plugin_input(app, pane, input, specs);
    process_outs(app, reply.outs, cmds, 1, specs);
}
