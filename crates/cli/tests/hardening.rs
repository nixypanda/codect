// Phase 7 hardening: packed objects, SHA-256 repositories, fatal projection
// failures, and output stability (TECHNICAL_DESIGN.md sections 16.4, 16.5, 20).
//
// Repositories are created with the `git` executable by the support module;
// the binary under test never invokes Git.

mod support;

use std::process::Output;

use support::{TestRepo, codect_in, stderr, stdout};

const RUST_BASE: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

const RUST_TYPE_VARIANT: &str = "\
pub struct User {
    pub id: u64,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

const ELM_BASE: &str = "\
module User exposing (..)

greet : String -> String
greet name =
    \"hi \" ++ name
";

const ELM_SIGNATURE_VARIANT: &str = "\
module User exposing (..)

greet : String -> Int
greet name =
    \"hi \" ++ name
";

// An unclosed parameter list guarantees a Tree-sitter `ERROR` (or missing)
// node, so projecting this file is fatal rather than a silent partial result.
const RUST_SYNTAX_ERROR: &str = "\
pub fn broken(
";

fn run(repo: &TestRepo, args: &[&str]) -> Output {
    codect_in(repo, args).output().expect("run codect")
}

fn run_with_columns(repo: &TestRepo, args: &[&str], columns: Option<&str>) -> Output {
    let mut command = codect_in(repo, args);
    match columns {
        Some(columns) => command.env("COLUMNS", columns),
        None => command.env_remove("COLUMNS"),
    };
    command.output().expect("run codect")
}

fn changed_repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("src/User.elm", ELM_BASE);
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/User.elm", ELM_SIGNATURE_VARIANT);
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("target");
    repo
}

fn has_pack_file(repo: &TestRepo) -> bool {
    std::fs::read_dir(repo.path().join(".git/objects/pack"))
        .expect("pack directory")
        .filter_map(Result::ok)
        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
}

fn loose_object_count(repo: &TestRepo) -> usize {
    std::fs::read_dir(repo.path().join(".git/objects"))
        .expect("objects directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.len() == 2 && name != "pack" && name != "info")
        })
        .flat_map(|entry| std::fs::read_dir(entry.path()).into_iter().flatten())
        .filter_map(Result::ok)
        .count()
}

// ---------------------------------------------------------------------------
// packed objects
// ---------------------------------------------------------------------------

#[test]
fn show_and_diff_read_packed_objects() {
    let repo = changed_repo();

    repo.git_ok(&["repack", "-a", "-d"]);
    // `repack` alone leaves loose copies behind; `prune-packed` removes them so
    // the reads below must come from the pack.
    repo.git_ok(&["prune-packed"]);

    assert!(
        has_pack_file(&repo),
        "repack should have produced a pack file"
    );
    assert_eq!(
        loose_object_count(&repo),
        0,
        "all objects should have been packed and pruned"
    );

    let show = run(&repo, &["show", "--mode", "signatures", "HEAD"]);
    assert!(show.status.success(), "stderr: {}", stderr(&show));
    assert!(
        show.stderr.is_empty(),
        "a successful show writes no diagnostics"
    );
    let document = stdout(&show);
    assert!(
        document.contains("== src/User.elm ==\n"),
        "packed Elm file missing: {document:?}"
    );
    assert!(
        document.contains("greet : String -> Int"),
        "packed Elm signature missing: {document:?}"
    );
    assert!(
        document.contains("pub struct User"),
        "packed Rust declaration missing: {document:?}"
    );

    let diff = run(&repo, &["diff", "--mode", "types", "HEAD~1", "HEAD"]);
    assert!(diff.status.success(), "stderr: {}", stderr(&diff));
    assert!(
        diff.stderr.is_empty(),
        "a successful diff writes no diagnostics"
    );
    let document = stdout(&diff);
    assert!(
        document.contains("-    pub id: u32,") && document.contains("+    pub id: u64,"),
        "packed Rust type change missing: {document:?}"
    );
}

// ---------------------------------------------------------------------------
// SHA-256 repositories
// ---------------------------------------------------------------------------

#[test]
fn show_works_in_a_sha256_repository_when_supported() {
    let Some(repo) = TestRepo::init_sha256() else {
        eprintln!("skipping SHA-256 test: the Git executable cannot create SHA-256 repositories");
        return;
    };

    repo.write("src/User.elm", ELM_BASE);
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("sha256 base");

    let show = run(&repo, &["show", "--mode", "signatures", "HEAD"]);
    assert!(show.status.success(), "stderr: {}", stderr(&show));
    assert!(
        show.stderr.is_empty(),
        "a successful show writes no diagnostics"
    );
    let document = stdout(&show);
    assert!(
        document.contains("== src/User.elm ==\n"),
        "SHA-256 Elm file missing: {document:?}"
    );
    assert!(
        document.contains("== src/lib.rs ==\n"),
        "SHA-256 Rust file missing: {document:?}"
    );
    assert!(
        document.contains("greet : String -> String"),
        "SHA-256 Elm signature missing: {document:?}"
    );
}

// ---------------------------------------------------------------------------
// fatal projection failures
// ---------------------------------------------------------------------------

#[test]
fn show_of_a_syntax_error_exits_one_without_partial_document() {
    let repo = TestRepo::init();
    // `src/aaa.rs` sorts first, so the valid projection is reached before the
    // failing one; a partial document must still never be written.
    repo.write("src/aaa.rs", RUST_BASE);
    repo.write("src/broken.rs", RUST_SYNTAX_ERROR);
    repo.commit("broken");

    let output = run(&repo, &["show", "--mode", "types", "HEAD"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "a fatal projection must not emit a partial document: {:?}",
        stdout(&output)
    );

    let diagnostic = stderr(&output);
    assert!(!diagnostic.is_empty(), "a diagnostic must go to stderr");
    assert!(
        diagnostic.contains("src/broken.rs"),
        "diagnostic must name the failing path: {diagnostic}"
    );
}

fn assert_diff_with_broken_side(broken_base: bool) {
    let repo = TestRepo::init();
    repo.write("src/aaa.rs", RUST_BASE);
    if broken_base {
        repo.write("src/broken.rs", RUST_SYNTAX_ERROR);
        repo.commit("broken base");
        repo.remove("src/broken.rs");
        repo.commit("valid target");
    } else {
        repo.commit("valid base");
        repo.write("src/broken.rs", RUST_SYNTAX_ERROR);
        repo.commit("broken target");
    }

    let output = run(&repo, &["diff", "--mode", "types", "HEAD~1", "HEAD"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "a fatal projection must not emit a partial diff: {:?}",
        stdout(&output)
    );

    let diagnostic = stderr(&output);
    assert!(!diagnostic.is_empty(), "a diagnostic must go to stderr");
    assert!(
        diagnostic.contains("src/broken.rs"),
        "diagnostic must name the failing path: {diagnostic}"
    );
}

#[test]
fn diff_with_a_syntax_error_on_either_side_exits_one_without_partial_document() {
    assert_diff_with_broken_side(false);
    assert_diff_with_broken_side(true);
}

// ---------------------------------------------------------------------------
// output stability
// ---------------------------------------------------------------------------

#[test]
fn show_and_diff_stdout_is_byte_identical_across_runs() {
    let repo = changed_repo();

    let show_args = ["show", "--mode", "signatures", "HEAD"];
    let first_show = stdout(&run(&repo, &show_args));
    let second_show = stdout(&run(&repo, &show_args));
    assert_eq!(first_show, second_show, "show output must be deterministic");
    assert!(
        !first_show.is_empty(),
        "the show fixture must be non-trivial"
    );

    let diff_args = ["diff", "--mode", "signatures", "HEAD~1", "HEAD"];
    let first_diff = stdout(&run(&repo, &diff_args));
    let second_diff = stdout(&run(&repo, &diff_args));
    assert_eq!(first_diff, second_diff, "diff output must be deterministic");
    assert!(
        !first_diff.is_empty(),
        "the diff fixture must be non-trivial"
    );
}

#[test]
fn stdout_is_independent_of_terminal_width() {
    let repo = changed_repo();

    let show_args = ["show", "--mode", "signatures", "HEAD"];
    let baseline_show = run_with_columns(&repo, &show_args, None);
    assert!(baseline_show.status.success());

    let diff_args = ["diff", "--mode", "signatures", "HEAD~1", "HEAD"];
    let baseline_diff = run_with_columns(&repo, &diff_args, None);
    assert!(baseline_diff.status.success());

    // `codect_in` captures stdout through a pipe, so every run below is already
    // non-terminal; varying `COLUMNS` proves width is not consulted either.
    for columns in ["20", "40", "80", "200", "1000"] {
        let show = run_with_columns(&repo, &show_args, Some(columns));
        assert_eq!(
            show.stdout, baseline_show.stdout,
            "show output changed at COLUMNS={columns}"
        );

        let diff = run_with_columns(&repo, &diff_args, Some(columns));
        assert_eq!(
            diff.stdout, baseline_diff.stdout,
            "diff output changed at COLUMNS={columns}"
        );
    }
}
