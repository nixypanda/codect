//! The `plugin_set!` macro: one struct, one message enum, one effect enum, and
//! one `PluginSet` impl per registry.
//!
//! The registry is compile-time: a plugin is a field, its message is an enum
//! variant, and its effect is an enum variant. Calls fan out in field order. A
//! `#[cfg]` on a field is copied onto the field, the enum variants, and every
//! call or arm for it, so a disabled feature compiles the plugin away.

#![allow(dead_code)]
/// Declare a plugin registry.
///
/// ```ignore
/// plugin_set! {
///     pub struct Plugins: PluginsMsg, PluginsCmd {
///         example: Example = some_plugin_crate::Example,
///     }
/// }
/// ```
///
/// The `field: Variant = Type` form names the enum variant explicitly, because
/// `macro_rules!` cannot upper-camel-case an identifier.
#[macro_export]
macro_rules! plugin_set {
    (
        $(#[$meta:meta])*
        pub struct $name:ident : $msg_name:ident, $cmd_name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $variant:ident = $ty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone)]
        pub struct $name {
            $(
                $(#[$field_meta])*
                pub $field: $ty,
            )*
        }

        $(#[$meta])*
        #[derive(Debug)]
        pub enum $msg_name {
            $(
                $(#[$field_meta])*
                $variant(<$ty as $crate::kernel::plugin::Plugin>::Msg),
            )*
        }

        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        pub enum $cmd_name {
            $(
                $(#[$field_meta])*
                $variant(<$ty as $crate::kernel::plugin::Plugin>::Cmd),
            )*
        }

        impl $name {
            /// Panics in debug builds when two plugins share an id.
            pub fn assert_unique_ids(&self) {
                let ids = $crate::kernel::plugin::PluginSet::ids(self);
                for (index, id) in ids.iter().enumerate() {
                    debug_assert!(
                        !ids[..index].contains(id),
                        "duplicate plugin id {id:?}",
                    );
                }
            }
        }

        impl $crate::kernel::plugin::PluginSet for $name {
            type Msg = $msg_name;
            type Cmd = $cmd_name;

            fn ids(&self) -> &'static [$crate::kernel::plugin::PluginId] {
                static IDS: &[$crate::kernel::plugin::PluginId] = &[
                    $( $(#[$field_meta])* <$ty as $crate::kernel::plugin::Plugin>::ID, )*
                ];
                IDS
            }

            fn panes(&self, cx: &$crate::kernel::cx::Cx) -> Vec<$crate::kernel::layout::PaneSpec> {
                let mut out = Vec::new();
                $( $(#[$field_meta])* out.extend($crate::kernel::plugin::Plugin::panes(&self.$field, cx)); )*
                out
            }

            fn has_focus(&self, focus: $crate::kernel::plugin::FocusKey) -> bool {
                let mut found = false;
                $( $(#[$field_meta])* { found |= $crate::kernel::plugin::Plugin::has_focus(&self.$field, focus); } )*
                found
            }

            fn first_focus(&self) -> Option<$crate::kernel::plugin::FocusKey> {
                let mut found = None;
                $(
                    $(#[$field_meta])*
                    if found.is_none() {
                        found = $crate::kernel::plugin::Plugin::first_focus(&self.$field);
                    }
                )*
                found
            }

            fn commands(&self, cx: &$crate::kernel::cx::Cx) -> Vec<$crate::kernel::command::CommandSpec> {
                let mut out = Vec::new();
                $( $(#[$field_meta])* out.extend($crate::kernel::plugin::Plugin::commands(&self.$field, cx)); )*
                out
            }

            fn help(&self, cx: &$crate::kernel::cx::Cx) -> Vec<$crate::kernel::help::HelpEntry> {
                let mut out = Vec::new();
                $( $(#[$field_meta])* out.extend($crate::kernel::plugin::Plugin::help(&self.$field, cx)); )*
                out
            }

            fn hints(
                &self,
                target: $crate::kernel::plugin::HintTarget,
                cx: &$crate::kernel::cx::Cx,
            ) -> Option<&'static str> {
                match target {
                    $crate::kernel::plugin::HintTarget::Overlay(pane) => {
                        $(
                            $(#[$field_meta])*
                            if $crate::kernel::plugin::Plugin::owns_pane(&self.$field, pane) {
                                return $crate::kernel::plugin::Plugin::hints(&self.$field, target, cx);
                            }
                        )*
                    }
                    $crate::kernel::plugin::HintTarget::Focus(focus) => {
                        $(
                            $(#[$field_meta])*
                            if $crate::kernel::plugin::Plugin::owns_focus(&self.$field, focus) {
                                return $crate::kernel::plugin::Plugin::hints(&self.$field, target, cx);
                            }
                        )*
                    }
                }
                None
            }

            fn chips(&self, cx: &$crate::kernel::cx::Cx) -> Vec<$crate::kernel::chrome::Chip> {
                let mut out = Vec::new();
                $( $(#[$field_meta])* out.extend($crate::kernel::plugin::Plugin::chips(&self.$field, cx)); )*
                out
            }

            fn status(&self, cx: &$crate::kernel::cx::Cx) -> Vec<$crate::kernel::chrome::Segment> {
                let mut out = Vec::new();
                $( $(#[$field_meta])* out.extend($crate::kernel::plugin::Plugin::status(&self.$field, cx)); )*
                out
            }

            fn overlay_spec(
                &self,
                pane: $crate::kernel::plugin::PaneId,
            ) -> $crate::kernel::plugin::OverlaySpec {
                $(
                    $(#[$field_meta])*
                    if $crate::kernel::plugin::Plugin::owns_pane(&self.$field, pane) {
                        return $crate::kernel::plugin::Plugin::overlay_spec(&self.$field, pane);
                    }
                )*
                $crate::kernel::plugin::OverlaySpec::MODAL
            }

            fn pane_owner(
                &self,
                pane: $crate::kernel::plugin::PaneId,
            ) -> Option<$crate::kernel::plugin::PluginId> {
                $(
                    $(#[$field_meta])*
                    if $crate::kernel::plugin::Plugin::owns_pane(&self.$field, pane) {
                        return Some(<$ty as $crate::kernel::plugin::Plugin>::ID);
                    }
                )*
                None
            }

            fn busy(&self) -> bool {
                let mut busy = false;
                $( $(#[$field_meta])* { busy |= $crate::kernel::plugin::Plugin::busy(&self.$field); } )*
                busy
            }

            fn render(
                &self,
                pane: $crate::kernel::plugin::PaneId,
                geometry: $crate::kernel::layout::PaneGeometry,
                focused: bool,
                cx: &$crate::kernel::cx::Cx,
                frame: &mut ratatui::Frame,
            ) {
                $( $(#[$field_meta])* $crate::kernel::plugin::Plugin::render(&self.$field, pane, geometry, focused, cx, &mut *frame); )*
            }

            fn render_overlay(
                &self,
                pane: $crate::kernel::plugin::PaneId,
                area: ratatui::layout::Rect,
                cx: &$crate::kernel::cx::Cx,
                frame: &mut ratatui::Frame,
            ) {
                $( $(#[$field_meta])* $crate::kernel::plugin::Plugin::render_overlay(&self.$field, pane, area, cx, &mut *frame); )*
            }

            fn input(
                &mut self,
                pane: $crate::kernel::plugin::PaneId,
                input: $crate::kernel::plugin::Input,
                cx: &$crate::kernel::cx::Cx,
            ) -> $crate::kernel::plugin::Reply<$cmd_name> {
                let mut handled = false;
                let mut outs = Vec::new();
                $(
                    $(#[$field_meta])*
                    {
                        let reply = $crate::kernel::plugin::Plugin::input(&mut self.$field, pane, input, cx);
                        handled |= reply.handled;
                        outs.extend(reply.outs.into_iter().map(|out| match out {
                            $crate::kernel::plugin::Out::Effect(cmd) => {
                                $crate::kernel::plugin::Out::Effect($cmd_name::$variant(cmd))
                            }
                            $crate::kernel::plugin::Out::Kernel(request) => {
                                $crate::kernel::plugin::Out::Kernel(request)
                            }
                        }));
                    }
                )*
                $crate::kernel::plugin::Reply { handled, outs }
            }

            fn run(
                &mut self,
                command: $crate::kernel::command::CommandId,
                cx: &$crate::kernel::cx::Cx,
            ) -> Vec<($crate::kernel::plugin::PluginId, $crate::kernel::plugin::Out<$cmd_name>)> {
                let mut outs = Vec::new();
                $(
                    $(#[$field_meta])*
                    {
                        let id = <$ty as $crate::kernel::plugin::Plugin>::ID;
                        outs.extend(
                            $crate::kernel::plugin::Plugin::run(&mut self.$field, command, cx)
                                .into_iter()
                                .map(|out| (id, match out {
                                    $crate::kernel::plugin::Out::Effect(cmd) => {
                                        $crate::kernel::plugin::Out::Effect($cmd_name::$variant(cmd))
                                    }
                                    $crate::kernel::plugin::Out::Kernel(request) => {
                                        $crate::kernel::plugin::Out::Kernel(request)
                                    }
                                })),
                        );
                    }
                )*
                outs
            }

            fn update(
                &mut self,
                msg: $msg_name,
                cx: &$crate::kernel::cx::Cx,
            ) -> Vec<($crate::kernel::plugin::PluginId, $crate::kernel::plugin::Out<$cmd_name>)> {
                match msg {
                    $(
                        $(#[$field_meta])*
                        $msg_name::$variant(msg) => {
                            let id = <$ty as $crate::kernel::plugin::Plugin>::ID;
                            $crate::kernel::plugin::Plugin::update(&mut self.$field, msg, cx)
                                .into_iter()
                                .map(|out| (id, match out {
                                    $crate::kernel::plugin::Out::Effect(cmd) => {
                                        $crate::kernel::plugin::Out::Effect($cmd_name::$variant(cmd))
                                    }
                                    $crate::kernel::plugin::Out::Kernel(request) => {
                                        $crate::kernel::plugin::Out::Kernel(request)
                                    }
                                }))
                                .collect()
                        }
                    )*
                }
            }

            fn on_event(
                &mut self,
                to: $crate::kernel::plugin::EventTarget,
                event: &$crate::kernel::plugin::Event,
                cx: &$crate::kernel::cx::Cx,
            ) -> Vec<($crate::kernel::plugin::PluginId, $crate::kernel::plugin::Out<$cmd_name>)> {
                let mut outs = Vec::new();
                $(
                    $(#[$field_meta])*
                    {
                        let id = <$ty as $crate::kernel::plugin::Plugin>::ID;
                        let deliver = match to {
                            $crate::kernel::plugin::EventTarget::All => true,
                            $crate::kernel::plugin::EventTarget::One(target) => target == id,
                        };
                        if deliver {
                            outs.extend(
                                $crate::kernel::plugin::Plugin::on_event(&mut self.$field, event, cx)
                                    .into_iter()
                                    .map(|out| (id, match out {
                                        $crate::kernel::plugin::Out::Effect(cmd) => {
                                            $crate::kernel::plugin::Out::Effect($cmd_name::$variant(cmd))
                                        }
                                        $crate::kernel::plugin::Out::Kernel(request) => {
                                            $crate::kernel::plugin::Out::Kernel(request)
                                        }
                                    })),
                            );
                        }
                    }
                )*
                outs
            }

            fn search(
                &mut self,
                needle: &str,
                cx: &$crate::kernel::cx::Cx,
            ) -> Vec<$crate::kernel::search::SearchMatch> {
                let mut out = Vec::new();
                $( $(#[$field_meta])* out.extend($crate::kernel::plugin::Plugin::search(&mut self.$field, needle, cx)); )*
                out
            }

            fn after_batch(
                &mut self,
                pending: &[$cmd_name],
                cx: &$crate::kernel::cx::Cx,
            ) -> Vec<($crate::kernel::plugin::PluginId, $crate::kernel::plugin::Out<$cmd_name>)> {
                let mut outs = Vec::new();
                $(
                    $(#[$field_meta])*
                    {
                        let id = <$ty as $crate::kernel::plugin::Plugin>::ID;
                        let mut own: Vec<<$ty as $crate::kernel::plugin::Plugin>::Cmd> = Vec::new();
                        #[allow(unreachable_code)]
                        for cmd in pending {
                            #[allow(irrefutable_let_patterns)]
                            if let $cmd_name::$variant(cmd) = cmd {
                                own.push(cmd.clone());
                            }
                        }
                        outs.extend(
                            $crate::kernel::plugin::Plugin::after_batch(&mut self.$field, &own, cx)
                                .into_iter()
                                .map(|out| (id, match out {
                                    $crate::kernel::plugin::Out::Effect(cmd) => {
                                        $crate::kernel::plugin::Out::Effect($cmd_name::$variant(cmd))
                                    }
                                    $crate::kernel::plugin::Out::Kernel(request) => {
                                        $crate::kernel::plugin::Out::Kernel(request)
                                    }
                                })),
                        );
                    }
                )*
                outs
            }

            fn settle(&mut self, cx: &$crate::kernel::cx::Cx) {
                $( $(#[$field_meta])* $crate::kernel::plugin::Plugin::settle(&mut self.$field, cx); )*
            }

            fn perform(engine: &engine::Engine, cmd: $cmd_name) -> $msg_name {
                match cmd {
                    $(
                        $(#[$field_meta])*
                        $cmd_name::$variant(cmd) => {
                            $msg_name::$variant(<$ty as $crate::kernel::plugin::Plugin>::perform(engine, cmd))
                        }
                    )*
                }
            }

            fn startup_failure(msg: $msg_name) -> Result<$msg_name, Box<engine::EngineError>> {
                match msg {
                    $(
                        $(#[$field_meta])*
                        $msg_name::$variant(msg) => {
                            <$ty as $crate::kernel::plugin::Plugin>::startup_failure(msg)
                                .map($msg_name::$variant)
                        }
                    )*
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::kernel::app::{App, Chrome, Kernel, Msg, update};
    use crate::kernel::cx::Cx;
    use crate::kernel::help::HelpEntry;
    use crate::kernel::keymap::Keymap;
    use crate::kernel::layout::{PaneSpec, Placement};
    use crate::kernel::plugin::{
        Event, EventTarget, FocusKey, Input, Out, OverlaySpec, PaneId, Plugin, PluginId, PluginSet,
        Request, Size,
    };
    use crate::kernel::workspace::{FileSet, Query, Scope, Workspace};
    use crate::render::icons::{IconStyle, Icons};
    use crate::render::theme::Theme;

    fn pane(owner: &'static str) -> PaneSpec {
        PaneSpec {
            id: PaneId {
                owner: PluginId(owner),
                slot: 0,
            },
            placement: Placement::Body,
            focus: FocusKey {
                owner: PluginId(owner),
                slot: 0,
            },
        }
    }

    #[derive(Clone)]
    pub struct First;
    impl Plugin for First {
        const ID: PluginId = PluginId("first");
        type Msg = ();
        type Cmd = ();
        fn panes(&self, _cx: &Cx) -> Vec<PaneSpec> {
            vec![pane("first")]
        }
        fn update(&mut self, _msg: (), _cx: &Cx) -> Vec<Out<()>> {
            vec![Out::Effect(())]
        }
        fn perform(_engine: &engine::Engine, _cmd: ()) {}
    }

    #[derive(Clone)]
    pub struct Second;
    impl Plugin for Second {
        const ID: PluginId = PluginId("second");
        type Msg = ();
        type Cmd = ();
        fn panes(&self, _cx: &Cx) -> Vec<PaneSpec> {
            vec![pane("second")]
        }
        fn help(&self, _cx: &Cx) -> Vec<HelpEntry> {
            vec![]
        }
        fn perform(_engine: &engine::Engine, _cmd: ()) {}
    }

    crate::plugin_set! {
        pub struct Order: OrderMsg, OrderCmd {
            first: First = First,
            second: Second = Second,
        }
    }

    crate::plugin_set! {
        pub struct Cfgd: CfgdMsg, CfgdCmd {
            first: First = First,
            #[cfg(any())]
            gone: Second = Second,
            #[cfg(all())]
            kept: Second = Second,
        }
    }

    #[derive(Clone)]
    pub struct DupOne;
    impl Plugin for DupOne {
        const ID: PluginId = PluginId("dup");
        type Msg = ();
        type Cmd = ();
        fn perform(_engine: &engine::Engine, _cmd: ()) {}
    }

    #[derive(Clone)]
    pub struct DupTwo;
    impl Plugin for DupTwo {
        const ID: PluginId = PluginId("dup");
        type Msg = ();
        type Cmd = ();
        fn perform(_engine: &engine::Engine, _cmd: ()) {}
    }

    crate::plugin_set! {
        pub struct Dupe: DupeMsg, DupeCmd {
            one: DupOne = DupOne,
            two: DupTwo = DupTwo,
        }
    }

    fn workspace() -> Workspace {
        Workspace {
            query: Query {
                mode: base::ProjectionMode::Types,
                scope: Scope {
                    selection: base::Selection::all(),
                    label: "all".to_owned(),
                },
            },
            files: FileSet::default(),
            selected: None,
            search: None,
        }
    }

    fn with_cx<R>(f: impl FnOnce(&Cx) -> R) -> R {
        let theme = Theme::dark();
        let icons = Icons::new(IconStyle::None);
        let workspace = workspace();
        let cx = Cx::new(
            Size {
                width: 100,
                height: 30,
            },
            &theme,
            &icons,
            "/repo",
            &workspace,
            None,
            crate::kernel::plugin::ViewId {
                owner: PluginId("first"),
                slot: 0,
            },
            None,
            None,
            false,
            30,
            false,
            &[],
            &[],
            &[],
        );
        f(&cx)
    }

    fn kernel() -> Kernel {
        Kernel {
            root: "/repo".to_owned(),
            size: Size {
                width: 100,
                height: 30,
            },
            chrome: Chrome {
                tree_percent: 30,
                theme: Theme::dark(),
                icons: Icons::new(IconStyle::None),
                spinner: 0,
                quit: false,
            },
            workspace: workspace(),
            focus: None,
            active_view: crate::kernel::plugin::ViewId {
                owner: PluginId("first"),
                slot: 0,
            },
            overlay: None,
            keymap: Keymap::default(),
            diagnostic: None,
        }
    }

    #[test]
    fn calls_fan_out_in_field_order() {
        let plugins = Order {
            first: First,
            second: Second,
        };
        let panes = with_cx(|cx| plugins.panes(cx));
        assert_eq!(panes, vec![pane("first"), pane("second")]);
    }

    #[test]
    fn cfg_fields_compile_away_or_in() {
        let plugins = Cfgd {
            first: First,
            kept: Second,
        };
        assert_eq!(
            plugins.ids(),
            &[PluginId("first"), PluginId("second")],
            "the cfg(any()) field is absent and cfg(all()) is present",
        );
    }

    #[test]
    fn outs_are_wrapped_with_the_owning_plugin_id() {
        let mut plugins = Order {
            first: First,
            second: Second,
        };
        let outs = with_cx(|cx| plugins.update(OrderMsg::First(()), cx));
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].0, PluginId("first"));
        assert!(matches!(outs[0].1, Out::Effect(OrderCmd::First(()))));
    }

    #[test]
    fn perform_dispatches_by_variant() {
        let engine = engine::Engine::discover(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("discover");
        let msg = Order::perform(&engine, OrderCmd::First(()));
        assert!(matches!(msg, OrderMsg::First(())));
    }

    #[test]
    fn defaults_are_empty() {
        let plugins = Order {
            first: First,
            second: Second,
        };
        with_cx(|cx| {
            assert!(plugins.commands(cx).is_empty());
            assert!(plugins.help(cx).is_empty());
            assert_eq!(
                plugins.hints(
                    crate::kernel::plugin::HintTarget::Focus(pane("first").focus),
                    cx,
                ),
                None
            );
            assert!(!plugins.busy());
            assert_eq!(plugins.overlay_spec(pane("first").id), OverlaySpec::MODAL);
        });
    }

    #[test]
    #[should_panic(expected = "duplicate plugin id")]
    fn duplicate_ids_panic() {
        let plugins = Dupe {
            one: DupOne,
            two: DupTwo,
        };
        plugins.assert_unique_ids();
    }

    #[test]
    #[should_panic(expected = "request cascade exceeded depth")]
    fn a_request_cascade_overflows_the_depth_limit() {
        #[derive(Clone)]
        pub struct Loopy;
        impl Plugin for Loopy {
            const ID: PluginId = PluginId("loopy");
            type Msg = ();
            type Cmd = ();
            fn update(&mut self, _msg: (), cx: &Cx) -> Vec<Out<()>> {
                vec![Out::Kernel(Request::QueryLoaded(
                    cx.workspace.query.clone(),
                ))]
            }
            fn on_event(&mut self, event: &Event, cx: &Cx) -> Vec<Out<()>> {
                match event {
                    Event::QueryChanged => {
                        vec![Out::Kernel(Request::QueryLoaded(
                            cx.workspace.query.clone(),
                        ))]
                    }
                    _ => Vec::new(),
                }
            }
            fn perform(_engine: &engine::Engine, _cmd: ()) {
                unreachable!()
            }
        }
        crate::plugin_set! {
            pub struct Loop: LoopMsg, LoopCmd {
                loopy: Loopy = Loopy,
            }
        }
        let app = App::new(kernel(), Loop { loopy: Loopy });
        let _ = update(Msg::<LoopMsg>::Plugin(LoopMsg::Loopy(())), &app);
    }

    #[test]
    fn eleven_field_registry_compiles() {
        crate::plugin_set! {
            pub struct Many: ManyMsg, ManyCmd {
                p1: P1 = First,
                p2: P2 = Second,
                p3: P3 = First,
                p4: P4 = Second,
                p5: P5 = First,
                p6: P6 = Second,
                p7: P7 = First,
                p8: P8 = Second,
                p9: P9 = First,
                p10: P10 = Second,
                p11: P11 = First,
            }
        }
        let plugins = Many {
            p1: First,
            p2: Second,
            p3: First,
            p4: Second,
            p5: First,
            p6: Second,
            p7: First,
            p8: Second,
            p9: First,
            p10: Second,
            p11: First,
        };
        with_cx(|cx| assert_eq!(plugins.panes(cx).len(), 11));
        let _ = (EventTarget::All, Input::Key(crate::util::input::Key::Esc));
    }
}
