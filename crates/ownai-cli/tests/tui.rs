//! CLI tests for the `tui` command surface.
//!
//! The terminal frontend is not driven here — these tests cover the command
//! boundary only: argument validation, and the non-terminal guard that keeps a
//! redirected invocation from emitting control sequences or writing stdout.

mod support;

use std::process::Output;

use support::{TestRepo, ownai, ownai_in, stderr};

const RUST_BASE: &str = "\
pub struct User {
    pub id: u32,
}
";

fn run(repo: &TestRepo, args: &[&str]) -> Output {
    ownai_in(repo, args).output().expect("run ownai")
}

fn repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo
}

/// A repository whose `HEAD` differs from its parent.
fn repo_with_change() -> TestRepo {
    let repo = repo();
    repo.write("src/lib.rs", "pub struct User {\n    pub id: u64,\n}\n");
    repo.commit("change");
    repo
}

#[test]
fn tui_show_without_a_terminal_exits_one_with_a_clean_stdout() {
    let repo = repo();

    let output = run(&repo, &["tui", "show", "--mode", "types"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout must stay empty, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains("terminal"),
        "expected a terminal diagnostic, got: {diagnostic}"
    );
}

#[test]
fn tui_show_defaults_the_revision_to_head() {
    let repo = repo();

    // Still non-terminal, so it must fail the same way without a revision
    // argument; this proves HEAD is accepted as the default.
    ownai_in(&repo, &["tui", "show", "--mode", "types"])
        .assert()
        .code(1);
}

#[test]
fn tui_show_without_a_mode_is_a_usage_error() {
    let repo = repo();
    ownai_in(&repo, &["tui", "show"]).assert().code(2);
}

#[test]
fn tui_accepts_the_icons_flag() {
    let repo = repo();
    // Accepted, then rejected only because stdout is not a terminal.
    ownai_in(
        &repo,
        &["tui", "show", "--mode", "types", "--icons", "nerd"],
    )
    .assert()
    .code(1);
    ownai_in(
        &repo,
        &["tui", "show", "--mode", "types", "--icons", "none"],
    )
    .assert()
    .code(1);
}

#[test]
fn an_unknown_icons_value_is_a_usage_error() {
    let repo = repo();
    ownai_in(
        &repo,
        &["tui", "show", "--mode", "types", "--icons", "bogus"],
    )
    .assert()
    .code(2);
}

#[test]
fn tui_without_a_subcommand_is_a_usage_error() {
    let repo = repo();
    ownai_in(&repo, &["tui"]).assert().code(2);
}

#[test]
fn tui_show_area_and_path_together_are_a_usage_error() {
    let repo = repo();
    ownai_in(
        &repo,
        &[
            "tui", "show", "--mode", "types", "-a", "frontend", "-p", "src",
        ],
    )
    .assert()
    .code(2);
}

#[test]
fn the_top_level_help_lists_the_tui_command() {
    let output = ownai().arg("--help").output().expect("run ownai");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");
    assert!(help.contains("tui"), "help must list `tui`: {help}");
}

#[test]
fn tui_diff_without_revisions_is_a_usage_error() {
    let repo = repo_with_change();
    ownai_in(&repo, &["tui", "diff", "range", "--mode", "types"])
        .assert()
        .code(2);
}

#[test]
fn bare_tui_diff_is_rejected_and_lists_both_views() {
    let repo = repo_with_change();
    let output = run(&repo, &["tui", "diff"]);
    assert_eq!(output.status.code(), Some(2));
    let diagnostic = stderr(&output);
    assert!(diagnostic.contains("range"), "{diagnostic}");
    assert!(diagnostic.contains("commits"), "{diagnostic}");
}

#[test]
fn commits_view_parses_then_requires_a_terminal() {
    let repo = repo_with_change();
    let output = run(
        &repo,
        &[
            "tui", "diff", "commits", "--mode", "types", "HEAD~1", "HEAD",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("terminal"));
}

#[test]
fn tui_diff_without_a_mode_is_a_usage_error() {
    let repo = repo_with_change();
    ownai_in(&repo, &["tui", "diff", "range", "HEAD~1", "HEAD"])
        .assert()
        .code(2);
}

#[test]
fn tui_diff_without_a_terminal_exits_one_with_a_clean_stdout() {
    let repo = repo_with_change();

    let output = run(
        &repo,
        &["tui", "diff", "range", "--mode", "types", "HEAD~1", "HEAD"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout must stay empty, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains("terminal"),
        "expected a terminal diagnostic, got: {diagnostic}"
    );
}
