//! The built-in plugins and the registry.
//!
//! Registration order is behaviour: events are broadcast in field order, and a
//! command's registration order breaks rank ties. The order is fixed here and
//! in section 4.5 of the plugin plan.

use crate::diff::{Cmd as DiffCmd, DiffPlugin};
use crate::finder::Finder;
use crate::help::Help;
use crate::kernel::app::{App, Chrome, Kernel};
use crate::kernel::keymap::Keymap;
use crate::kernel::plugin::Size;
use crate::kernel::workspace::{FileSet, NO_CHANGES, NO_FILES, Query, Scope, Workspace};
use crate::mode_picker::ModePicker;
use crate::palette::Palette;
use crate::plugin::{COMPARE_VIEW, HISTORY_VIEW, SOURCE_VIEW, TREE_FOCUS};
use crate::prompt::Prompt;
use crate::render::icons::Icons;
use crate::render::theme::Theme;
use crate::route::{DiffRequest, DiffView, LoadRequest, ShowRequest};
use crate::scope_chooser::ScopeChooser;
use crate::search_input::SearchInput;
use crate::source::{Cmd as SourceCmd, SourceView};
use crate::tree::Tree;
use crate::which_key::WhichKey;

crate::plugin_set! {
    /// Every plugin `codect tui` ships, in registration order.
    pub struct Plugins: PluginsMsg, PluginsCmd {
        tree: Tree = crate::tree::Tree,
        which_key: WhichKey = crate::which_key::WhichKey,
        prompt: Prompt = crate::prompt::Prompt,
        help: Help = crate::help::Help,
        palette: Palette = crate::palette::Palette,
        finder: Finder = crate::finder::Finder,
        search: Search = crate::search_input::SearchInput,
        modes: Modes = crate::mode_picker::ModePicker,
        scope: Scope = crate::scope_chooser::ScopeChooser,
        source: Source = crate::source::SourceView,
        diff: Diff = crate::diff::DiffPlugin,
    }
}

/// The options the kernel is built from.
pub struct KernelOptions {
    pub root: String,
    pub request: LoadRequest,
    pub scope_label: String,
    pub width: u16,
    pub height: u16,
    pub theme: Theme,
    pub icons: Icons,
}

/// Build the registry and the kernel, plus the startup effect.
pub fn build(options: KernelOptions) -> (App<Plugins>, Vec<PluginsCmd>) {
    let KernelOptions {
        root,
        request,
        scope_label,
        width,
        height,
        theme,
        icons,
    } = options;

    let empty = match &request {
        LoadRequest::Show { .. } => NO_FILES,
        LoadRequest::Diff { .. } => NO_CHANGES,
    };
    let query = match &request {
        LoadRequest::Show {
            mode, selection, ..
        }
        | LoadRequest::Diff {
            mode, selection, ..
        } => Query {
            mode: *mode,
            scope: Scope {
                selection: selection.clone(),
                label: scope_label,
            },
        },
    };

    let (source, diff, active_view, startup) = match request {
        LoadRequest::Show {
            revision,
            mode,
            selection,
        } => (
            SourceView::new(revision.clone(), mode),
            DiffPlugin::inactive(),
            SOURCE_VIEW,
            PluginsCmd::Source(SourceCmd::Load(ShowRequest {
                revision,
                mode,
                selection,
            })),
        ),
        LoadRequest::Diff {
            base,
            target,
            mode,
            selection,
            view,
            merge_base,
        } => {
            let request = DiffRequest {
                base: base.clone(),
                target: target.clone(),
                mode,
                selection,
                view,
                merge_base,
            };
            let cmd = match view {
                DiffView::Range => DiffCmd::Range(request),
                DiffView::Commits => DiffCmd::History(request),
            };
            let active_view = match view {
                DiffView::Range => COMPARE_VIEW,
                DiffView::Commits => HISTORY_VIEW,
            };
            (
                SourceView::inactive(),
                DiffPlugin::new(base, target, mode, view, merge_base),
                active_view,
                PluginsCmd::Diff(cmd),
            )
        }
    };

    let tree = Tree::default();
    let which_key = WhichKey::default();
    let prompt = Prompt::default();
    let help = Help;
    let palette = Palette::default();
    let finder = Finder::default();
    let search = SearchInput::default();
    let modes = ModePicker::default();
    let scope = ScopeChooser::default();
    let focus = Some(TREE_FOCUS);
    let workspace = Workspace {
        query,
        files: FileSet {
            empty,
            ..FileSet::default()
        },
        selected: None,
        search: None,
    };
    let plugins = Plugins {
        tree,
        which_key,
        prompt,
        help,
        palette,
        finder,
        search,
        modes,
        scope,
        source,
        diff,
    };
    plugins.assert_unique_ids();
    let kernel = Kernel {
        root,
        size: Size { width, height },
        chrome: Chrome {
            tree_percent: 30,
            theme,
            icons,
            spinner: 0,
            quit: false,
        },
        workspace,
        focus,
        active_view,
        overlay: None,
        keymap: Keymap::default(),
        diagnostic: None,
    };
    (App::new(kernel, plugins), vec![startup])
}
