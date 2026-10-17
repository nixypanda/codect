//! Golden frames: the byte-stable rendering oracle for the TUI refactor.
//!
//! `#[cfg(test)]` only. Each scenario builds an `App` through the same
//! `update`/`settle`/`press` path the runtime uses, renders it to a
//! `TestBackend` buffer, and compares the result with a committed file under
//! `crates/tui/tests/golden/<name>.txt`. `UPDATE_GOLDEN=1` rewrites those
//! files; every later refactor task must keep them byte-identical unless it
//! lists an intentional change.
//!
//! The scenarios never set model fields by hand: state that a scenario needs
//! (focus, cursor placement, overlays, paging) is reached with `press` for key
//! sequences and `step` for synthesized messages. Themes are always built
//! explicitly, never detected, so the frames do not depend on the environment.

use std::fs;
use std::path::Path;

use base::{FileDiff, ProjectionMode, Selection, SelectionError, SourceOutline};
use engine::EngineError;

use crate::api::Cmd;
use crate::kernel::app::{Msg, settle, update};
use crate::plugins::PluginsCmd;
use crate::plugins::legacy::Msg as LegacyMsg;
use crate::render::icons::{IconStyle, Icons};
use crate::render::theme::{Capability, Flavor, Theme};
use crate::route::LoadRequest;
use crate::test_support::{
    ATTRIBUTED_SOURCE, App, NESTED_SOURCE, after_batch_effects, area_set, attributed_child_outline,
    base_app_with, commits_app, diff_app, diff_request, dropped_container_outline, file_diff,
    golden_document, legacy_msg, nested_outline, press, render, show_app, show_app_mode,
    show_request, source_outline, step, tests_filter_outline, two_files,
};
use crate::util::input::Key;

/// One golden frame: a name, the render size, and a builder that returns the
/// settled model to render.
struct Scenario {
    name: &'static str,
    width: u16,
    height: u16,
    build: fn() -> App,
}

#[test]
fn golden_frames() {
    let regenerate = std::env::var_os("UPDATE_GOLDEN").is_some();
    let directory = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));

    for scenario in scenarios() {
        let app = (scenario.build)();

        // Internal nondeterminism guard: two renders of the same model must be
        // byte-identical before either is compared with the committed file.
        let first = golden_document(
            scenario.name,
            scenario.width,
            scenario.height,
            &render(&app, scenario.width, scenario.height),
        );
        let second = golden_document(
            scenario.name,
            scenario.width,
            scenario.height,
            &render(&app, scenario.width, scenario.height),
        );
        assert_eq!(
            first, second,
            "scenario `{}` renders nondeterministically",
            scenario.name
        );

        let path = directory.join(format!("{}.txt", scenario.name));
        if regenerate {
            write_atomic(&path, &first);
            continue;
        }

        let committed = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read {}: {error}\nrun `UPDATE_GOLDEN=1 cargo test -p tui golden` to generate it",
                path.display()
            )
        });
        assert_eq!(
            committed,
            first,
            "scenario `{}` differs from its golden file\n{}\nrun `UPDATE_GOLDEN=1 cargo test -p tui golden` to regenerate",
            scenario.name,
            line_diff(&committed, &first),
        );
    }
}

fn scenarios() -> Vec<Scenario> {
    vec![
        scenario("show_initial", 100, 30, build_show_initial),
        scenario("show_fold_opened", 100, 30, build_show_fold_opened),
        scenario("show_folds", 100, 30, build_show_folds),
        scenario("show_dropped", 100, 30, build_show_dropped),
        scenario("show_tests_mode", 100, 30, build_show_tests_mode),
        scenario("show_narrow_tree", 60, 20, build_show_narrow_tree),
        scenario("show_narrow_body", 60, 20, build_show_narrow_body),
        scenario("show_palette", 100, 30, build_show_palette),
        scenario(
            "show_palette_filtered",
            100,
            30,
            build_show_palette_filtered,
        ),
        scenario("show_finder", 100, 30, build_show_finder),
        scenario("show_finder_many", 100, 30, build_show_finder_many),
        scenario("show_search", 100, 30, build_show_search),
        scenario("show_which_key", 100, 30, build_show_which_key),
        scenario("show_which_key_short", 100, 12, build_show_which_key_short),
        scenario("show_help", 100, 40, build_show_help),
        scenario("show_mode_picker", 100, 30, build_show_mode_picker),
        scenario("show_scope_chooser", 100, 30, build_show_scope_chooser),
        scenario("show_revision_prompt", 100, 30, build_show_revision_prompt),
        scenario("show_diagnostic", 100, 30, build_show_diagnostic),
        scenario("show_busy", 100, 30, build_show_busy),
        scenario("show_icons", 100, 30, build_show_icons),
        scenario("diff_range", 120, 30, build_diff_range),
        scenario("diff_range_narrow", 60, 24, build_diff_range_narrow),
        scenario("diff_range_search", 120, 30, build_diff_range_search),
        scenario("diff_base_prompt", 120, 30, build_diff_base_prompt),
        scenario("diff_merge_base", 120, 30, build_diff_merge_base),
        scenario("diff_commits", 120, 30, build_diff_commits),
        scenario("diff_commits_narrow", 60, 24, build_diff_commits_narrow),
        scenario("diff_commits_step", 120, 30, build_diff_commits_step),
        scenario("too_small", 30, 6, build_too_small),
        scenario("too_small_overlay", 30, 6, build_too_small_overlay),
        scenario("light_theme", 100, 30, build_light_theme),
        scenario("show_fold_attributes", 100, 30, build_show_fold_attributes),
        scenario("diff_badges", 120, 30, build_diff_badges),
        scenario("show_which_key_run", 100, 30, build_show_which_key_run),
        scenario("show_search_wrap", 100, 30, build_show_search_wrap),
        scenario("layout_w39", 39, 20, build_layout_w39),
        scenario("layout_w40", 40, 20, build_layout_w40),
        scenario("layout_w79", 79, 20, build_layout_w79),
        scenario("layout_w80", 80, 20, build_layout_w80),
        scenario("layout_h7", 100, 7, build_layout_h7),
        scenario("layout_h8", 100, 8, build_layout_h8),
        scenario("no_color", 100, 30, build_no_color),
        scenario("show_reload_last", 100, 30, build_show_reload_last),
    ]
}

const fn scenario(name: &'static str, width: u16, height: u16, build: fn() -> App) -> Scenario {
    Scenario {
        name,
        width,
        height,
        build,
    }
}

/// Resize through `update` so `settle` sees the size the frame is drawn at.
fn resize(app: &App, width: u16, height: u16) -> App {
    step(app, Msg::Resize { width, height })
}

fn two_source_files() -> Vec<SourceOutline> {
    vec![source_outline("a.rs", "a\n"), source_outline("b.rs", "b\n")]
}

fn load_show(theme: Theme, icons: Icons, files: Vec<SourceOutline>) -> App {
    let app = base_app_with(
        LoadRequest::Show {
            revision: "HEAD".to_owned(),
            mode: ProjectionMode::Types,
            selection: Selection::all(),
        },
        theme,
        icons,
    );
    step(
        &app,
        legacy_msg(LegacyMsg::ShowLoaded {
            request: show_request(),
            result: Ok(files.into()),
        }),
    )
}

fn three_diffs() -> Vec<FileDiff> {
    vec![
        file_diff("added.rs", None, Some("added\n")),
        file_diff("changed.rs", Some("before\n"), Some("after\n")),
        file_diff("deleted.rs", Some("deleted\n"), None),
    ]
}

fn build_show_initial() -> App {
    two_files()
}

/// `za` on the method: the child opens, so every source line is visible.
fn build_show_fold_opened() -> App {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    press(
        &app,
        &[Key::Tab, Key::Char('j'), Key::Char('z'), Key::Char('a')],
    )
}

/// The default folded state: the cursor rests on the method's collapsed row,
/// which renders its signature with the `⋯ N lines` suffix.
fn build_show_folds() -> App {
    let app = show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )]);
    press(&app, &[Key::Tab, Key::Char('j')])
}

fn build_show_dropped() -> App {
    show_app_mode(
        vec![dropped_container_outline("a.rs", NESTED_SOURCE)],
        ProjectionMode::Signatures,
    )
}

fn build_show_tests_mode() -> App {
    show_app_mode(vec![tests_filter_outline()], ProjectionMode::Tests)
}

fn build_show_narrow_tree() -> App {
    resize(&two_files(), 60, 20)
}

fn build_show_narrow_body() -> App {
    press(&resize(&two_files(), 60, 20), &[Key::Tab])
}

fn build_show_palette() -> App {
    press(&two_files(), &[Key::CtrlP])
}

fn build_show_palette_filtered() -> App {
    press(
        &two_files(),
        &[
            Key::CtrlP,
            Key::Char('f'),
            Key::Char('o'),
            Key::Char('l'),
            Key::Char('d'),
        ],
    )
}

fn build_show_finder() -> App {
    press(&two_files(), &[Key::CtrlF, Key::Char('b')])
}

/// Three files and a query that matches all of them: the finder popup clips its
/// last row, so at least one real result row is still visible.
fn build_show_finder_many() -> App {
    let app = show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("b.rs", "b\n"),
        source_outline("c.rs", "c\n"),
    ]);
    press(
        &app,
        &[Key::CtrlF, Key::Char('.'), Key::Char('r'), Key::Char('s')],
    )
}

fn build_show_search() -> App {
    press(&two_files(), &[Key::Char('/'), Key::Char('a'), Key::Enter])
}

fn build_show_which_key() -> App {
    press(&two_files(), &[Key::Tab, Key::Char('z')])
}

fn build_show_which_key_short() -> App {
    press(&resize(&two_files(), 100, 12), &[Key::Tab, Key::Char('z')])
}

fn build_show_help() -> App {
    press(&resize(&two_files(), 100, 40), &[Key::Char('?')])
}

fn build_show_mode_picker() -> App {
    press(&two_files(), &[Key::Char('m')])
}

fn build_show_scope_chooser() -> App {
    step(
        &press(&two_files(), &[Key::Char('s')]),
        legacy_msg(LegacyMsg::AreasLoaded(Ok(area_set()))),
    )
}

fn build_show_revision_prompt() -> App {
    press(&two_files(), &[Key::Char('r')])
}

fn build_show_diagnostic() -> App {
    let error = EngineError::Selection(SelectionError::UnknownArea {
        name: "x".to_owned(),
    });
    step(
        &two_files(),
        legacy_msg(LegacyMsg::ShowLoaded {
            request: show_request(),
            result: Err(Box::new(error)),
        }),
    )
}

fn build_show_busy() -> App {
    press(&two_files(), &[Key::Char('m'), Key::Enter])
}

fn build_show_icons() -> App {
    load_show(
        Theme::dark(),
        Icons::new(IconStyle::Nerd),
        two_source_files(),
    )
}

fn build_diff_range() -> App {
    resize(&diff_app(three_diffs()), 120, 30)
}

fn build_diff_range_narrow() -> App {
    press(&resize(&diff_app(three_diffs()), 60, 24), &[Key::Tab])
}

fn build_diff_range_search() -> App {
    let app = resize(
        &diff_app(vec![file_diff(
            "a.rs",
            Some("one\ntwo\n"),
            Some("one\n2\n"),
        )]),
        120,
        30,
    );
    press(
        &app,
        &[
            Key::Char('/'),
            Key::Char('o'),
            Key::Char('n'),
            Key::Char('e'),
            Key::Enter,
        ],
    )
}

fn build_diff_base_prompt() -> App {
    press(
        &resize(&diff_app(three_diffs()), 120, 30),
        &[Key::Char('b')],
    )
}

fn build_diff_merge_base() -> App {
    let mut request = diff_request();
    request.merge_base = true;
    let app = base_app_with(
        LoadRequest::Diff {
            base: request.base.clone(),
            target: request.target.clone(),
            mode: request.mode,
            selection: request.selection.clone(),
            view: request.view,
            merge_base: request.merge_base,
        },
        Theme::dark(),
        Icons::new(IconStyle::None),
    );
    let app = step(
        &app,
        legacy_msg(LegacyMsg::DiffLoaded {
            request,
            result: Ok(three_diffs().into()),
        }),
    );
    resize(&app, 120, 30)
}

fn build_diff_commits() -> App {
    resize(&commits_app(5), 120, 30)
}

fn build_diff_commits_narrow() -> App {
    resize(&commits_app(5), 60, 24)
}

fn build_diff_commits_step() -> App {
    let app = press(&resize(&commits_app(5), 120, 30), &[Key::Char('j')]);
    // The step effect is coalesced by the runtime, not by `update`; run it here
    // so the scenario exercises the real step-loading path.
    let mut model = app.clone();
    let cmds = after_batch_effects(&mut model, &Vec::new());
    let index = cmds
        .iter()
        .find_map(|cmd| match cmd {
            PluginsCmd::Legacy(Cmd::Step { index, .. }) => Some(*index),
            _ => None,
        })
        .expect("a commit step effect");
    let result = Ok(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into());
    settle(update(legacy_msg(LegacyMsg::StepLoaded { index, result }), &model).0)
}

fn build_too_small() -> App {
    resize(&two_files(), 30, 6)
}

fn build_too_small_overlay() -> App {
    press(&resize(&two_files(), 30, 6), &[Key::Char('?')])
}

fn build_light_theme() -> App {
    load_show(
        Theme::new(Flavor::Light, Capability::TrueColor),
        Icons::new(IconStyle::None),
        two_source_files(),
    )
}

fn build_show_fold_attributes() -> App {
    // The attributed declaration sits in the folded child; the attribute is only
    // visible once that child is opened, so the cursor is placed on the child
    // row and `za` toggles the child (not the enclosing container).
    let app = show_app(vec![attributed_child_outline("a.rs", ATTRIBUTED_SOURCE)]);
    press(
        &app,
        &[Key::Tab, Key::Char('j'), Key::Char('z'), Key::Char('a')],
    )
}

fn build_diff_badges() -> App {
    let diffs = vec![
        file_diff("a.rs", None, Some("a\n")),
        file_diff("b.rs", Some("b\n"), Some("B\n")),
        file_diff("c.rs", Some("c\n"), None),
    ];
    resize(&diff_app(diffs), 120, 30)
}

fn build_show_which_key_run() -> App {
    press(&two_files(), &[Key::Tab, Key::Char('z'), Key::Char('a')])
}

fn build_show_search_wrap() -> App {
    let app = show_app(vec![source_outline("a.rs", "a\nb\na\n")]);
    press(
        &app,
        &[
            Key::Char('/'),
            Key::Char('a'),
            Key::Enter,
            Key::Char('n'),
            Key::Char('n'),
        ],
    )
}

fn build_layout_w39() -> App {
    resize(&two_files(), 39, 20)
}

fn build_layout_w40() -> App {
    resize(&two_files(), 40, 20)
}

fn build_layout_w79() -> App {
    resize(&two_files(), 79, 20)
}

fn build_layout_w80() -> App {
    resize(&two_files(), 80, 20)
}

fn build_layout_h7() -> App {
    resize(&two_files(), 100, 7)
}

fn build_layout_h8() -> App {
    resize(&two_files(), 100, 8)
}

fn build_no_color() -> App {
    load_show(
        Theme::new(Flavor::Dark, Capability::NoColor),
        Icons::new(IconStyle::None),
        two_source_files(),
    )
}

fn build_show_reload_last() -> App {
    let app = press(&two_files(), &[Key::Char('j')]);
    step(
        &app,
        legacy_msg(LegacyMsg::ShowLoaded {
            request: show_request(),
            result: Ok(vec![source_outline("a.rs", "a\n")].into()),
        }),
    )
}

fn write_atomic(path: &Path, contents: &str) {
    let directory = path.parent().expect("the golden directory");
    fs::create_dir_all(directory).expect("create the golden directory");
    let temp = path.with_extension("txt.tmp");
    fs::write(&temp, contents).expect("write the temporary golden file");
    fs::rename(&temp, path).expect("replace the golden file");
}

fn line_diff(committed: &str, rendered: &str) -> String {
    let committed: Vec<&str> = committed.lines().collect();
    let rendered: Vec<&str> = rendered.lines().collect();
    let mut report = String::new();
    for index in 0..committed.len().max(rendered.len()) {
        let expected = committed.get(index).copied().unwrap_or("<missing>");
        let actual = rendered.get(index).copied().unwrap_or("<missing>");
        if expected != actual {
            report.push_str(&format!(
                "line {}:\n  committed: {expected}\n  rendered:  {actual}\n",
                index + 1,
            ));
        }
    }
    report
}
