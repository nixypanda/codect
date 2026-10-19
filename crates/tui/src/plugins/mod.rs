//! The built-in plugins and the registry.
//!
//! Tasks 4.3–4.9 extract the rest and add them to the registry in field order.

pub mod legacy;

use crate::api::Cmd;
use crate::help::Help;
use crate::kernel::app::{App, Chrome, Kernel};
use crate::kernel::keymap::Keymap;
use crate::kernel::workspace::{FileSet, NO_CHANGES, NO_FILES, Query, Scope, Workspace};
use crate::plugin::TREE_FOCUS;
use crate::prompt::Prompt;
use crate::render::icons::Icons;
use crate::render::theme::Theme;
use crate::route::{DiffRequest, LoadRequest, ShowRequest};
use crate::tree::Tree;
use crate::which_key::WhichKey;

pub use legacy::Legacy;

crate::plugin_set! {
    /// Every plugin `codect tui` ships, in registration order.
    pub struct Plugins: PluginsMsg, PluginsCmd {
        tree: Tree = crate::tree::Tree,
        which_key: WhichKey = crate::which_key::WhichKey,
        prompt: Prompt = crate::prompt::Prompt,
        help: Help = crate::help::Help,
        legacy: Legacy = crate::plugins::legacy::Legacy,
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

    let legacy = Legacy::new(request.clone());
    let tree = Tree::default();
    let which_key = WhichKey::default();
    let prompt = Prompt::default();
    let help = Help;
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
        legacy,
    };
    plugins.assert_unique_ids();
    let kernel = Kernel {
        root,
        size: crate::kernel::plugin::Size { width, height },
        chrome: Chrome {
            tree_percent: 30,
            theme,
            icons,
            spinner: 0,
            quit: false,
        },
        workspace,
        focus,
        overlay: None,
        keymap: Keymap::default(),
        diagnostic: None,
    };
    let startup = vec![PluginsCmd::Legacy(request_into_cmd(request))];
    (App::new(kernel, plugins), startup)
}

/// The effect a session starts with.
pub fn request_into_cmd(request: LoadRequest) -> Cmd {
    match request {
        LoadRequest::Show {
            revision,
            mode,
            selection,
        } => Cmd::Show(ShowRequest {
            revision,
            mode,
            selection,
        }),
        LoadRequest::Diff {
            base,
            target,
            mode,
            selection,
            view,
            merge_base,
        } => Cmd::Diff(DiffRequest {
            base,
            target,
            mode,
            selection,
            view,
            merge_base,
        }),
    }
}
