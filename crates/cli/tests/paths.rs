// End-to-end coverage of `--path`/`-p` scoping for `show` and `diff`
// (TECHNICAL_DESIGN.md sections 8.2, 14, 16.5).
//
// Repositories are created with the `git` executable by the support module;
// the binary under test never invokes Git.

mod support;

use std::process::Output;

use support::{TestRepo, ownai, ownai_in, stderr, stdout};

const RUST_BASE: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

const RUST_BODY_VARIANT: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hello {name}\")
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

fn run(repo: &TestRepo, args: &[&str]) -> Output {
    ownai_in(repo, args).output().expect("run ownai")
}

fn sections(document: &str) -> Vec<&str> {
    document
        .lines()
        .filter(|line| line.starts_with("== ") && line.ends_with(" =="))
        .collect()
}

fn repo_with_type_change() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("type change");
    repo
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

#[test]
fn show_scoped_to_a_directory_omits_sibling_directories() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(&repo, &["show", "--mode", "types", "-p", "src/alpha"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(sections(&stdout(&output)), ["== src/alpha/lib.rs =="]);
}

#[test]
fn show_scoped_to_a_single_file_emits_one_section() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(
        &repo,
        &["show", "--mode", "types", "-p", "src/alpha/lib.rs"],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(sections(&stdout(&output)), ["== src/alpha/lib.rs =="]);
}

#[test]
fn repeated_paths_union_in_raw_path_byte_order() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_BASE);
    repo.write("src/gamma/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(
        &repo,
        &[
            "show",
            "--mode",
            "types",
            "-p",
            "src/beta",
            "-p",
            "src/alpha",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(
        sections(&stdout(&output)),
        ["== src/alpha/lib.rs ==", "== src/beta/lib.rs =="]
    );
}

#[test]
fn dot_inside_a_subdirectory_scopes_to_that_directory() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = ownai()
        .current_dir(repo.path().join("src/alpha"))
        .args(["show", "--mode", "types", "-p", "."])
        .output()
        .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(sections(&stdout(&output)), ["== src/alpha/lib.rs =="]);
}

#[test]
fn absolute_path_inside_the_repository_scopes_to_it() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_BASE);
    repo.commit("base");

    // The temporary directory is a symlink on macOS, so canonicalize the
    // test-side path to match the working directory the OS hands the child.
    let absolute = std::fs::canonicalize(repo.path().join("src/alpha")).expect("canonical path");
    let output = ownai_in(&repo, &["show", "--mode", "types", "--path"])
        .arg(&absolute)
        .output()
        .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(sections(&stdout(&output)), ["== src/alpha/lib.rs =="]);
}

#[test]
fn dot_at_the_repository_root_is_equivalent_to_no_scoping() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("base");

    let unscoped = run(&repo, &["show", "--mode", "types"]);
    let scoped = run(&repo, &["show", "--mode", "types", "-p", "."]);
    assert!(unscoped.status.success(), "stderr: {}", stderr(&unscoped));
    assert!(scoped.status.success(), "stderr: {}", stderr(&scoped));
    assert!(
        !stdout(&unscoped).is_empty(),
        "the fixture must produce a non-trivial document"
    );
    assert_eq!(stdout(&unscoped), stdout(&scoped));
}

#[test]
fn parent_beyond_the_repository_root_is_a_fatal_diagnostic() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(&repo, &["show", "--mode", "types", "-p", ".."]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("outside"),
        "diagnostic missing the out-of-repository cause: {}",
        stderr(&output)
    );
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[test]
fn diff_scoped_to_a_file_omits_other_changed_files() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.write("src/other.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.write("src/other.rs", RUST_TYPE_VARIANT);
    repo.commit("type change");

    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "types",
            "-p",
            "src/lib.rs",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let document = stdout(&output);
    assert!(
        document.contains("diff --ownai a/src/lib.rs b/src/lib.rs"),
        "scoped file block missing: {document:?}"
    );
    assert!(
        !document.contains("src/other.rs"),
        "an out-of-scope changed file must not appear: {document:?}"
    );
}

#[test]
fn diff_scoped_to_a_directory_omits_other_changed_directories() {
    let repo = TestRepo::init();
    repo.write("src/alpha/lib.rs", RUST_BASE);
    repo.write("src/beta/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/alpha/lib.rs", RUST_TYPE_VARIANT);
    repo.write("src/beta/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("type change");

    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "types",
            "-p",
            "src/alpha",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let document = stdout(&output);
    assert!(
        document.contains("diff --ownai a/src/alpha/lib.rs b/src/alpha/lib.rs"),
        "scoped directory block missing: {document:?}"
    );
    assert!(
        !document.contains("src/beta"),
        "an out-of-scope directory must not appear: {document:?}"
    );
}

#[test]
fn scoped_diff_marks_added_and_deleted_files_with_dev_null() {
    let repo = TestRepo::init();
    repo.write("src/alpha/gone.rs", RUST_BASE);
    repo.commit("base");
    repo.remove("src/alpha/gone.rs");
    repo.write("src/alpha/new.rs", RUST_BASE);
    repo.commit("target");

    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "types",
            "-p",
            "src/alpha",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let document = stdout(&output);
    assert!(
        document.contains(
            "diff --ownai a/src/alpha/new.rs b/src/alpha/new.rs\n--- /dev/null\n+++ b/src/alpha/new.rs\n"
        ),
        "added file block missing: {document:?}"
    );
    assert!(
        document.contains(
            "diff --ownai a/src/alpha/gone.rs b/src/alpha/gone.rs\n--- a/src/alpha/gone.rs\n+++ /dev/null\n"
        ),
        "deleted file block missing: {document:?}"
    );
}

#[test]
fn scoping_preserves_body_only_invisibility() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_BODY_VARIANT);
    repo.commit("body only");

    for mode in ["types", "signatures"] {
        ownai_in(
            &repo,
            &["diff", "--mode", mode, "-p", "src/lib.rs", "HEAD~1", "HEAD"],
        )
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(predicates::str::is_empty());
    }
}

// ---------------------------------------------------------------------------
// non-UTF-8 paths
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn show_targets_a_non_utf8_committed_path_on_unix() {
    use std::os::unix::ffi::OsStrExt as _;

    let repo = TestRepo::init();
    let relative = std::ffi::OsStr::from_bytes(b"src/\xFF/lib.rs");
    let absolute = repo.path().join(relative);
    let prepared = std::fs::create_dir_all(absolute.parent().expect("parent"))
        .and_then(|()| std::fs::write(&absolute, RUST_BASE));
    if prepared.is_err() {
        // APFS rejects invalid UTF-8 names (EILSEQ); Linux filesystems accept
        // them, so check the capability at runtime like the SHA-256 case.
        eprintln!("skipping non-UTF-8 path test: the filesystem rejects invalid UTF-8 names");
        return;
    }
    repo.commit("non-utf8 path");

    let output = ownai()
        .current_dir(repo.path())
        .args(["show", "--mode", "types", "--path"])
        .arg(relative)
        .output()
        .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("== src/\\xFF/lib.rs =="),
        "escaped non-UTF-8 path missing: {:?}",
        stdout(&output)
    );
}

// ---------------------------------------------------------------------------
// color
// ---------------------------------------------------------------------------

#[test]
fn scoped_redirected_output_contains_no_escape_bytes() {
    let repo = repo_with_type_change();

    let show = run(
        &repo,
        &[
            "--color=never",
            "show",
            "--mode",
            "types",
            "-p",
            "src/lib.rs",
        ],
    );
    assert!(show.status.success(), "stderr: {}", stderr(&show));
    assert!(!stdout(&show).contains('\u{1b}'));

    let diff = run(
        &repo,
        &[
            "--color=never",
            "diff",
            "--mode",
            "types",
            "-p",
            "src/lib.rs",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(diff.status.success(), "stderr: {}", stderr(&diff));
    assert!(!stdout(&diff).contains('\u{1b}'));
}
