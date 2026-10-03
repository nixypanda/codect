// End-to-end CLI tests over real temporary Git repositories
// (TECHNICAL_DESIGN.md sections 8.2, 14, 16.5).
//
// Each test asserts stdout, stderr, and the exit status for a documented
// command form. Repositories are created with the `git` executable by the
// support module; the binary under test never invokes Git.

mod support;

use std::process::Output;

use support::{TestRepo, codect, codect_in, fixture, stderr, stdout};

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

const RUST_SIGNATURE_VARIANT: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str, excited: bool) -> String {
    format!(\"hi {name}\")
}
";

const RUST_TYPE_VARIANT: &str = "\
pub struct User {
    pub id: u64,
}

pub fn greet(name: &str, excited: bool) -> String {
    format!(\"hi {name}\")
}
";

const ELM_BASE: &str = "\
module User exposing (..)

greet : String -> String
greet name =
    \"hi \" ++ name
";

const ELM_BODY_VARIANT: &str = "\
module User exposing (..)

greet : String -> String
greet name =
    \"hello \" ++ name
";

fn run(repo: &TestRepo, args: &[&str]) -> Output {
    codect_in(repo, args).output().expect("run codect")
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
fn show_elm_only_defaults_to_head_in_both_modes() {
    let repo = TestRepo::init();
    repo.write("src/User.elm", &fixture("elm/type-aliases/input.elm"));
    repo.commit("base");

    let types = format!(
        "== src/User.elm ==\n{}",
        fixture("elm/type-aliases/types.txt")
    );
    let signatures = format!(
        "== src/User.elm ==\n{}",
        fixture("elm/type-aliases/signatures.txt")
    );

    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success()
        .stdout(predicates::str::diff(types.clone()))
        .stderr(predicates::str::is_empty());
    codect_in(&repo, &["show", "--mode", "signatures"])
        .assert()
        .success()
        .stdout(predicates::str::diff(signatures));
    codect_in(&repo, &["show", "--mode", "types", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::diff(types));
}

#[test]
fn show_wraps_a_long_python_signature_within_the_line_budget() {
    let repo = TestRepo::init();
    repo.write("src/service.py", &fixture("python/line-wrapping/input.py"));
    repo.commit("base");

    let expected = format!(
        "== src/service.py ==\n{}",
        fixture("python/line-wrapping/signatures.txt")
    );
    codect_in(&repo, &["show", "--mode", "signatures"])
        .assert()
        .success()
        .stdout(predicates::str::diff(expected))
        .stderr(predicates::str::is_empty());
}

#[test]
fn show_wraps_a_nested_rust_return_type_within_the_line_budget() {
    let repo = TestRepo::init();
    repo.write("src/registry.rs", &fixture("rust/nested-return/input.rs"));
    repo.commit("base");

    let expected = format!(
        "== src/registry.rs ==\n{}",
        fixture("rust/nested-return/signatures.txt")
    );
    codect_in(&repo, &["show", "--mode", "signatures"])
        .assert()
        .success()
        .stdout(predicates::str::diff(expected))
        .stderr(predicates::str::is_empty());
}

#[test]
fn show_all_four_languages_in_byte_order() {
    let repo = TestRepo::init();
    repo.write("src/App.elm", &fixture("elm/normal-module/input.elm"));
    repo.write("src/Main.hs", &fixture("haskell/canonical-types/input.hs"));
    repo.write("src/app.py", &fixture("python/canonical-types/input.py"));
    repo.write("src/lib.rs", &fixture("rust/structs/input.rs"));
    repo.commit("base");

    let expected = format!(
        "== src/App.elm ==\n{}\n== src/Main.hs ==\n{}\n== src/app.py ==\n{}\n== src/lib.rs ==\n{}",
        fixture("elm/normal-module/types.txt"),
        fixture("haskell/canonical-types/types.txt"),
        fixture("python/canonical-types/types.txt"),
        fixture("rust/structs/types.txt"),
    );

    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success()
        .stdout(predicates::str::diff(expected))
        .stderr(predicates::str::is_empty());
}

#[test]
fn focused_diff_hides_body_changes_across_haskell_and_python() {
    let repo = TestRepo::init();
    repo.write(
        "src/Model.hs",
        "module Model where\n\narea :: Int -> Int\narea width = width * 2\n",
    );
    repo.write(
        "src/model.py",
        "def area(width: int) -> int:\n    return width * 2\n",
    );
    repo.commit("base");
    repo.write(
        "src/Model.hs",
        "module Model where\n\narea :: Int -> Int\narea width = width * 3\n",
    );
    repo.write(
        "src/model.py",
        "def area(width: int) -> int:\n    return width * 3\n",
    );
    repo.commit("body only");

    codect_in(&repo, &["diff", "--mode", "signatures", "HEAD^", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(predicates::str::is_empty());
}

#[test]
fn show_rust_only_in_both_modes() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", &fixture("rust/structs/input.rs"));
    repo.commit("base");

    let types = format!("== src/lib.rs ==\n{}", fixture("rust/structs/types.txt"));
    let signatures = format!(
        "== src/lib.rs ==\n{}",
        fixture("rust/structs/signatures.txt")
    );

    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success()
        .stdout(predicates::str::diff(types));
    codect_in(&repo, &["show", "--mode", "signatures"])
        .assert()
        .success()
        .stdout(predicates::str::diff(signatures));
}

#[test]
fn show_mixed_repository_orders_by_raw_path_bytes() {
    let repo = TestRepo::init();
    repo.write("src/App.elm", &fixture("elm/normal-module/input.elm"));
    repo.write("src/lib.rs", &fixture("rust/structs/input.rs"));
    repo.commit("base");

    // "src/App.elm" sorts before "src/lib.rs" because 'A' < 'l'.
    let expected = format!(
        "== src/App.elm ==\n{}\n== src/lib.rs ==\n{}",
        fixture("elm/normal-module/types.txt"),
        fixture("rust/structs/types.txt")
    );

    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success()
        .stdout(predicates::str::diff(expected));
}

#[test]
fn show_accepts_an_explicit_full_object_id() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", &fixture("rust/canonical-signatures/input.rs"));
    let id = repo.commit("base");

    let expected = format!(
        "== src/lib.rs ==\n{}",
        fixture("rust/canonical-signatures/signatures.txt")
    );

    codect_in(&repo, &["show", "--mode", "signatures", &id])
        .assert()
        .success()
        .stdout(predicates::str::diff(expected));
}

#[test]
fn show_empty_supported_file_set_is_empty_and_successful() {
    let repo = TestRepo::init();
    repo.write("README.md", "not a supported source file\n");
    repo.commit("docs");

    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(predicates::str::is_empty());
}

#[test]
fn show_path_with_no_supported_files_is_empty_and_successful() {
    let repo = TestRepo::init();
    repo.write("docs/readme.md", "not a supported source file\n");
    repo.commit("docs");

    // The directory exists in the revision, so the empty result is real rather
    // than a mistyped `--path`.
    codect_in(&repo, &["show", "--mode", "types", "--path", "docs"])
        .assert()
        .success()
        .stdout(predicates::str::is_empty())
        .stderr(predicates::str::is_empty());
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[test]
fn diff_reports_added_deleted_and_modified_files_and_skips_unchanged() {
    let repo = TestRepo::init();
    let keep = fixture("elm/normal-module/input.elm");
    repo.write("src/keep.elm", &keep);
    repo.write("src/mod.elm", &fixture("elm/type-aliases/input.elm"));
    repo.write("src/gone.rs", &fixture("rust/unions-aliases/input.rs"));
    repo.write("README.md", "ignored\n");
    repo.commit("base");

    repo.write("src/new.rs", &fixture("rust/structs/input.rs"));
    repo.write("src/mod.elm", &fixture("elm/canonical-types/input.elm"));
    repo.remove("src/gone.rs");
    repo.commit("target");

    let output = run(&repo, &["diff", "--mode", "types", "HEAD~1", "HEAD"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        output.stderr.is_empty(),
        "successful diff must not write diagnostics"
    );

    let document = stdout(&output);
    assert!(
        document
            .contains("diff --codect a/src/new.rs b/src/new.rs\n--- /dev/null\n+++ b/src/new.rs\n"),
        "added file block missing: {document:?}"
    );
    assert!(
        document.contains(
            "diff --codect a/src/gone.rs b/src/gone.rs\n--- a/src/gone.rs\n+++ /dev/null\n"
        ),
        "deleted file block missing: {document:?}"
    );
    assert!(
        document.contains(
            "diff --codect a/src/mod.elm b/src/mod.elm\n--- a/src/mod.elm\n+++ b/src/mod.elm\n"
        ),
        "modified file block missing: {document:?}"
    );
    assert!(
        !document.contains("a/src/keep.elm"),
        "an unchanged projection must not appear: {document:?}"
    );

    // Blocks are emitted in raw path byte order.
    let gone = document.find("a/src/gone.rs").expect("gone.rs block");
    let modified = document.find("a/src/mod.elm").expect("mod.elm block");
    let new = document.find("a/src/new.rs").expect("new.rs block");
    assert!(gone < modified && modified < new, "order: {document:?}");
}

#[test]
fn body_only_change_is_empty_and_successful_in_both_modes() {
    let rust = TestRepo::init();
    rust.write("src/lib.rs", RUST_BASE);
    rust.commit("base");
    rust.write("src/lib.rs", RUST_BODY_VARIANT);
    rust.commit("body only");

    let elm = TestRepo::init();
    elm.write("src/User.elm", ELM_BASE);
    elm.commit("base");
    elm.write("src/User.elm", ELM_BODY_VARIANT);
    elm.commit("body only");

    for mode in ["types", "signatures"] {
        for repo in [&rust, &elm] {
            codect_in(repo, &["diff", "--mode", mode, "HEAD~1", "HEAD"])
                .assert()
                .success()
                .stdout(predicates::str::is_empty())
                .stderr(predicates::str::is_empty());
        }
    }
}

#[test]
fn signature_change_appears_only_in_signatures() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_SIGNATURE_VARIANT);
    repo.commit("signature change");

    codect_in(&repo, &["diff", "--mode", "types", "HEAD~1", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::is_empty());

    codect_in(&repo, &["diff", "--mode", "signatures", "HEAD~1", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "-pub fn greet(name: &str) -> String;",
        ))
        .stdout(predicates::str::contains(
            "+pub fn greet(name: &str, excited: bool) -> String;",
        ));
}

#[test]
fn type_change_appears_in_both_modes() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("type change");

    for mode in ["types", "signatures"] {
        let output = run(&repo, &["diff", "--mode", mode, "HEAD~1", "HEAD"]);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
        let document = stdout(&output);
        assert!(
            document.contains("-    pub id: u32,") && document.contains("+    pub id: u64,"),
            "type change missing in {mode}: {document:?}"
        );
    }
}

#[test]
fn diff_path_deleted_in_target_still_succeeds() {
    let repo = TestRepo::init();
    repo.write("src/gone.rs", RUST_BASE);
    repo.commit("base");
    repo.remove("src/gone.rs");
    repo.commit("remove");

    // The path exists only in the base revision; the target's deletion is the
    // change being projected, so the scope must not be rejected.
    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "types",
            "--path",
            "src/gone.rs",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        output.stderr.is_empty(),
        "successful diff must not write diagnostics"
    );
    assert!(
        stdout(&output).contains("diff --codect a/src/gone.rs b/src/gone.rs"),
        "deleted file block missing: {:?}",
        stdout(&output)
    );
}

// ---------------------------------------------------------------------------
// named areas
// ---------------------------------------------------------------------------

fn repo_with_areas() -> TestRepo {
    let repo = TestRepo::init();
    repo.write(
        "apps/web/src/App.elm",
        &fixture("elm/type-aliases/input.elm"),
    );
    repo.write(
        "libs/ui/src/Widget.elm",
        &fixture("elm/normal-module/input.elm"),
    );
    repo.write("services/api/src/lib.rs", RUST_BASE);
    repo.write(
        ".codect.toml",
        "[areas]\nfrontend = [\"apps/web\", \"libs/ui\"]\nbackend = [\"services/api\"]\n",
    );
    repo.commit("base");
    repo
}

#[test]
fn show_with_an_area_scopes_to_its_paths() {
    let repo = repo_with_areas();

    let expected = format!(
        "== apps/web/src/App.elm ==\n{}\n== libs/ui/src/Widget.elm ==\n{}",
        fixture("elm/type-aliases/types.txt"),
        fixture("elm/normal-module/types.txt")
    );

    codect_in(&repo, &["show", "--mode", "types", "--area", "frontend"])
        .assert()
        .success()
        .stdout(predicates::str::diff(expected))
        .stderr(predicates::str::is_empty());
}

#[test]
fn repeated_areas_are_a_union() {
    let repo = repo_with_areas();

    let document = run(
        &repo,
        &["show", "--mode", "types", "-a", "frontend", "-a", "backend"],
    );
    assert!(document.status.success(), "stderr: {}", stderr(&document));

    let document = stdout(&document);
    assert!(
        document.contains("== apps/web/src/App.elm =="),
        "{document:?}"
    );
    assert!(
        document.contains("== services/api/src/lib.rs =="),
        "{document:?}"
    );
}

#[test]
fn area_paths_are_relative_to_the_repository_root_from_a_subdirectory() {
    let repo = repo_with_areas();

    // Run from inside the area itself: a working-directory-relative `--path`
    // would resolve to `apps/web/apps/web` and fail, so this proves `--area`
    // ignores the working directory.
    let output = codect()
        .current_dir(repo.path().join("apps/web"))
        .args(["show", "--mode", "types", "-a", "frontend"])
        .output()
        .expect("run codect");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("== apps/web/src/App.elm =="),
        "{:?}",
        stdout(&output)
    );
}

#[test]
fn an_area_is_satisfied_when_any_of_its_paths_exists() {
    let repo = TestRepo::init();
    repo.write(
        "apps/web/src/App.elm",
        &fixture("elm/type-aliases/input.elm"),
    );
    repo.write(
        ".codect.toml",
        "[areas]\nfrontend = [\"apps/web\", \"gone\"]\n",
    );
    repo.commit("base");

    codect_in(&repo, &["show", "--mode", "types", "-a", "frontend"])
        .assert()
        .success()
        .stdout(predicates::str::contains("== apps/web/src/App.elm =="));
}

#[test]
fn an_unknown_area_exits_one_and_lists_the_known_names() {
    let repo = repo_with_areas();

    let output = run(&repo, &["show", "--mode", "types", "-a", "ghost"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");

    let diagnostic = stderr(&output);
    assert!(diagnostic.contains("ghost"), "diagnostic: {diagnostic}");
    assert!(diagnostic.contains("frontend"), "diagnostic: {diagnostic}");
    assert!(diagnostic.contains("backend"), "diagnostic: {diagnostic}");
}

#[test]
fn an_area_absent_from_the_revision_exits_one() {
    let repo = TestRepo::init();
    repo.write("src/User.elm", &fixture("elm/type-aliases/input.elm"));
    repo.write(".codect.toml", "[areas]\nfrontend = [\"apps/web\"]\n");
    repo.commit("base");

    // `frontend` is defined, but none of its paths exist in the revision, so
    // the group is unsatisfied and the diagnostic names the area.
    let output = run(&repo, &["show", "--mode", "types", "-a", "frontend"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("area: frontend"),
        "diagnostic: {}",
        stderr(&output)
    );
}

#[test]
fn a_missing_config_exits_one() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(&repo, &["show", "--mode", "types", "-a", "frontend"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains(".codect.toml"),
        "diagnostic: {}",
        stderr(&output)
    );
}

#[test]
fn a_malformed_config_exits_one_only_for_area_selection() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.write(".codect.toml", "this is not toml\n");
    repo.commit("base");

    let output = run(&repo, &["show", "--mode", "types", "-a", "frontend"]);
    assert_eq!(output.status.code(), Some(1));

    // A malformed config must not affect `--path` or an unscoped run.
    codect_in(&repo, &["show", "--mode", "types", "--path", "src/lib.rs"])
        .assert()
        .success()
        .stdout(predicates::str::contains("== src/lib.rs =="));
    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success();
}

#[test]
fn area_and_path_together_are_a_usage_error() {
    let repo = repo_with_areas();

    codect_in(
        &repo,
        &["show", "--mode", "types", "-a", "frontend", "-p", "src"],
    )
    .assert()
    .code(2);
}

// ---------------------------------------------------------------------------
// bare repositories
// ---------------------------------------------------------------------------

#[test]
fn show_and_diff_work_in_a_bare_repository() {
    let normal = TestRepo::init();
    normal.write("src/User.elm", &fixture("elm/type-aliases/input.elm"));
    normal.commit("base");
    normal.write("src/User.elm", &fixture("elm/canonical-types/input.elm"));
    normal.commit("change");

    let bare = normal.clone_bare();

    codect_in(&bare, &["show", "--mode", "types", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::contains("== src/User.elm =="));

    codect_in(&bare, &["diff", "--mode", "types", "HEAD~1", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::contains("diff --codect a/src/User.elm"));
}

// ---------------------------------------------------------------------------
// failures and exit codes
// ---------------------------------------------------------------------------

#[test]
fn invalid_revision_exits_one_with_a_diagnostic_and_empty_stdout() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(&repo, &["show", "--mode", "types", "no-such-revision"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("no-such-revision"),
        "diagnostic missing the revision: {}",
        stderr(&output)
    );
}

#[test]
fn show_path_absent_from_the_revision_exits_one() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    let output = run(
        &repo,
        &["show", "--mode", "types", "--path", "src/missing.rs"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("src/missing.rs"),
        "diagnostic missing the path: {}",
        stderr(&output)
    );
}

#[test]
fn diff_path_absent_from_both_revisions_exits_one() {
    let repo = repo_with_type_change();

    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "types",
            "--path",
            "src/missing.rs",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("src/missing.rs"),
        "diagnostic missing the path: {}",
        stderr(&output)
    );
}

#[test]
fn a_range_passed_as_one_revision_exits_one() {
    let repo = repo_with_type_change();

    let output = run(&repo, &["diff", "--mode", "types", "HEAD~1..HEAD", "HEAD"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("range"),
        "diagnostic missing range context: {}",
        stderr(&output)
    );
}

#[test]
fn missing_mode_is_a_usage_error() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");

    codect_in(&repo, &["show", "HEAD"]).assert().code(2);
    codect_in(&repo, &["diff", "HEAD~1", "HEAD"])
        .assert()
        .code(2);
}

#[test]
fn missing_diff_revisions_are_usage_errors() {
    let repo = repo_with_type_change();

    codect_in(&repo, &["diff", "--mode", "types", "HEAD"])
        .assert()
        .code(2);
    codect_in(&repo, &["diff", "--mode", "types"])
        .assert()
        .code(2);
}

// ---------------------------------------------------------------------------
// color
// ---------------------------------------------------------------------------

#[test]
fn color_never_and_redirected_output_contain_no_escape_bytes() {
    let repo = repo_with_type_change();

    let never = run(
        &repo,
        &["--color=never", "diff", "--mode", "types", "HEAD~1", "HEAD"],
    );
    assert!(never.status.success());
    assert!(
        !stdout(&never).contains('\u{1b}'),
        "`--color=never` must not emit ANSI"
    );

    // Default `auto` with captured (non-terminal) stdout must also strip ANSI.
    let auto = run(&repo, &["diff", "--mode", "types", "HEAD~1", "HEAD"]);
    assert!(auto.status.success());
    assert!(
        !stdout(&auto).contains('\u{1b}'),
        "redirected output must not emit ANSI"
    );

    let show = run(&repo, &["--color=never", "show", "--mode", "types"]);
    assert!(show.status.success());
    assert!(!stdout(&show).contains('\u{1b}'));
}

#[test]
fn color_always_contains_escape_bytes() {
    let repo = repo_with_type_change();

    let diff = run(
        &repo,
        &[
            "--color=always",
            "diff",
            "--mode",
            "types",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(diff.status.success());
    assert!(
        stdout(&diff).contains('\u{1b}'),
        "`--color=always` must emit ANSI for a diff"
    );

    let show = run(&repo, &["--color=always", "show", "--mode", "types"]);
    assert!(show.status.success());
    assert!(
        stdout(&show).contains('\u{1b}'),
        "`--color=always` must emit ANSI for section headers"
    );
}
