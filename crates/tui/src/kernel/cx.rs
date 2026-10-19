//! `Cx`: the read-only view of kernel state a plugin sees, plus the collected
//! declarations.
//!
//! It is rebuilt from kernel state for every plugin call, but the collected
//! `panes`, `commands`, and `help` are borrowed from the per-step cache, so a
//! request cascade never re-collects them.

#![allow(dead_code)]
use crate::kernel::command::{CommandSpec, PrefixGroupView};
use crate::kernel::focus::focus_default;
use crate::kernel::help::HelpEntry;
use crate::kernel::keymap;
use crate::kernel::layout::{
    BodyLayout, MIN_HEIGHT, PaneGeometry, PaneSpec, Placement, SIDE_BY_SIDE_MIN_WIDTH,
    SINGLE_PANE_MIN_WIDTH, body_layout, frame_areas,
};
use crate::kernel::plugin::{FocusKey, PaneId, PluginId, Size};
use crate::kernel::workspace::Workspace;
use crate::render::icons::Icons;
use crate::render::theme::Theme;
use ratatui::layout::Rect;

pub struct Cx<'a> {
    pub size: Size,
    pub theme: &'a Theme,
    pub icons: &'a Icons,
    pub root: &'a str,
    pub workspace: &'a Workspace,
    pub focus: Option<FocusKey>,
    pub overlay: Option<PaneId>,
    pub pending_prefix: Option<char>,
    pub busy: bool,
    pub navigator_percent: u16,
    pub has_diagnostic: bool,
    panes: &'a [PaneSpec],
    commands: &'a [CommandSpec],
    help: &'a [HelpEntry],
}

impl<'a> Cx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        size: Size,
        theme: &'a Theme,
        icons: &'a Icons,
        root: &'a str,
        workspace: &'a Workspace,
        focus: Option<FocusKey>,
        overlay: Option<PaneId>,
        pending_prefix: Option<char>,
        busy: bool,
        navigator_percent: u16,
        has_diagnostic: bool,
        panes: &'a [PaneSpec],
        commands: &'a [CommandSpec],
        help: &'a [HelpEntry],
    ) -> Self {
        Self {
            size,
            theme,
            icons,
            root,
            workspace,
            focus,
            overlay,
            pending_prefix,
            busy,
            navigator_percent,
            has_diagnostic,
            panes,
            commands,
            help,
        }
    }

    /// Whether `owner`'s view is the active one. In this phase each plugin's
    /// declarations are always active.
    pub fn is_active_view(&self, owner: PluginId) -> bool {
        let _ = owner;
        true
    }

    pub fn commands(&self) -> &[CommandSpec] {
        self.commands
    }

    pub fn help_rows(&self) -> &[HelpEntry] {
        self.help
    }

    /// The palette commands: `palette == true`, ordered by `(rank, order)`.
    pub fn palette_commands(&self) -> Vec<&CommandSpec> {
        let mut specs: Vec<&CommandSpec> =
            self.commands.iter().filter(|spec| spec.palette).collect();
        specs.sort_by_key(|spec| spec.rank);
        specs
    }

    /// The which-key band's data: the in-scope commands whose binding is
    /// `Prefixed(prefix, _)`, in `(rank, registration)` order.
    pub fn prefix_group(&self, prefix: char) -> Option<PrefixGroupView> {
        let (group, bindings) = keymap::prefix_group(self.commands, prefix, self.focus)?;
        Some(PrefixGroupView {
            key: group.prefix,
            name: group.name,
            bindings: bindings
                .into_iter()
                .map(|binding| (binding.key, binding.label, binding.id))
                .collect(),
        })
    }

    /// The body layout with the current focus.
    pub fn layout(&self) -> BodyLayout {
        self.layout_with(self.focus)
    }

    /// The body layout at a given focus.
    pub fn layout_if_focused(&self, focus: FocusKey) -> BodyLayout {
        self.layout_with(Some(focus))
    }

    /// The body layout in an already-solved content rect. `view` solves the
    /// frame once for the chrome, so reusing that content keeps the per-draw
    /// frame solve to one instead of two.
    pub fn layout_in(&self, content: Rect) -> BodyLayout {
        self.layout_in_with(content, self.focus)
    }

    fn layout_with(&self, focus: Option<FocusKey>) -> BodyLayout {
        let (_, content, _) = frame_areas(self.size.width, self.size.height);
        self.layout_in_with(content, focus)
    }

    fn layout_in_with(&self, content: Rect, focus: Option<FocusKey>) -> BodyLayout {
        body_layout(
            content,
            self.navigator_percent,
            self.panes,
            focus,
            self.theme,
        )
    }

    /// The geometry of `pane` in the current layout, if it is visible.
    pub fn geometry(&self, pane: PaneId) -> Option<PaneGeometry> {
        self.layout()
            .slots
            .into_iter()
            .find(|slot| slot.pane == pane)
            .map(|slot| slot.geometry)
    }

    /// Whether the body is visible: not too small, and (wide enough or the
    /// focus is not a navigator's).
    pub fn body_visible(&self) -> bool {
        if self.size.width < SINGLE_PANE_MIN_WIDTH || self.size.height < MIN_HEIGHT {
            return false;
        }
        if self.size.width >= SIDE_BY_SIDE_MIN_WIDTH {
            return true;
        }
        let navigator = self.focus.is_some_and(|focus| {
            self.panes
                .iter()
                .any(|spec| spec.placement == Placement::Navigator && spec.focus == focus)
        });
        !navigator
    }

    /// The default focus for the visible panes.
    pub fn default_focus(&self) -> Option<FocusKey> {
        focus_default(self.panes)
    }
}
