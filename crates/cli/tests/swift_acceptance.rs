// End-to-end acceptance for the fifth language, Swift.
//
// These are black-box tests over real temporary Git repositories: disk show,
// stdin, explicit revisions, snapshots (`:index`, `:worktree`, `:empty`), text
// and JSON, path/area scoping, untracked files, exact and fuzzy moves,
// body-only empty diffs, atomic failure with a valid file selected alongside an
// invalid one, and the active JSON schemas. The binary under test never invokes
// Git; repository setup uses the `git` executable in the support module.
//
// Expected projections come from the frozen contract in
// `docs/plans/swift-language-contract.md`.

mod support;

use std::process::Output;

use serde_json::Value;
use support::{TestRepo, codect, codect_in, doc, stderr, stdout};

const POINT: &str = "\
struct Point: Equatable {
    var x: Double
    var y: Double = 0
    static let origin: Point = Point(x: 0, y: 0)
    func shifted(by dx: Double) -> Point { Point(x: x + dx, y: y) }
}
";

const POINT_TYPES: &str = "\
struct Point: Equatable {
    var x: Double
    var y: Double
    static let origin: Point
}
";

const POINT_SIGNATURES: &str = "\
struct Point: Equatable {
    var x: Double
    var y: Double
    static let origin: Point
    func shifted(by dx: Double) -> Point { ... }
}
";

const SWIFT_TESTS: &str = "\
@Test func basic() {}

@Suite struct MathTests {
    @Test func add(a: Int, b: Int) {}
}

final class FooTests: XCTestCase {
    func testThing() {}
}

extension FooTests {
    func testExtension() {}
}
";

const SWIFT_TESTS_GOLDEN: &str = "\
@Test func basic() { ... }

@Suite struct MathTests {
    @Test func add(a: Int, b: Int) { ... }
}

final class FooTests: XCTestCase {
    func testThing() { ... }
}

extension FooTests {
    func testExtension() { ... }
}
";

// A cleanly unclosed container: the grammar reports a MISSING node, which is
// fatal like any other selected projection failure.
const BROKEN_SWIFT: &str = "struct Broken {\n    var x: Int\n";

fn run(repo: &TestRepo, args: &[&str]) -> Output {
    codect_in(repo, args).output().expect("run codect")
}

fn run_json(repo: &TestRepo, args: &[&str]) -> Value {
    let output = run(repo, args);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains('\u{1b}'),
        "JSON must not contain ANSI"
    );
    assert!(
        stdout(&output).ends_with('\n'),
        "JSON must end with a newline"
    );
    serde_json::from_str(&stdout(&output)).expect("stdout is a JSON document")
}

fn show_json_stdin(repo: &TestRepo, path: &str, mode: &str, source: &str) -> Value {
    let output = codect_in(
        repo,
        &[
            "show", "--format", "json", "--stdin", "--path", path, "--mode", mode,
        ],
    )
    .write_stdin(source)
    .output()
    .expect("run codect");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    serde_json::from_str(&stdout(&output)).expect("stdout is a JSON document")
}

fn validate_show(document: &Value) {
    let schema: Value = serde_json::from_str(&doc("schema/codect.show.v1.json"))
        .expect("show schema is valid JSON");
    support::schema::validate(&schema, document)
        .unwrap_or_else(|error| panic!("show document is not schema-valid: {error}"));
}

fn validate_diff(document: &Value) {
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json"))
        .expect("diff schema is valid JSON");
    support::schema::validate(&schema, document)
        .unwrap_or_else(|error| panic!("diff document is not schema-valid: {error}"));
}

fn files(document: &Value) -> &[Value] {
    document["files"].as_array().expect("files array")
}

fn statuses(document: &Value) -> Vec<&str> {
    files(document)
        .iter()
        .map(|file| file["status"].as_str().expect("status string"))
        .collect()
}

fn file_at<'a>(document: &'a Value, path: &str) -> &'a Value {
    files(document)
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("no file event for {path:?}: {document}"))
}

fn projection_text(file: &Value) -> &str {
    file["projection"]["text"]
        .as_str()
        .expect("projection text")
}

fn paths(document: &Value) -> Vec<&str> {
    files(document)
        .iter()
        .map(|file| file["path"].as_str().expect("path"))
        .collect()
}

#[test]
fn swift_help_names_the_fifth_language() {
    let output = codect().arg("--help").output().expect("run codect --help");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let help = stdout(&output);
    assert!(help.contains("Swift"), "long help must name Swift: {help}");
    assert!(
        help.contains("revisions or disk"),
        "long help must reflect disk show: {help}"
    );
}

#[test]
fn swift_disk_show_is_textual_and_schema_valid_json() {
    let repo = TestRepo::init();
    repo.write("src/Point.swift", POINT);
    repo.commit("base");

    codect_in(&repo, &["show", "--mode", "types"])
        .assert()
        .success()
        .stdout(predicates::str::diff(format!(
            "== src/Point.swift ==\n{POINT_TYPES}"
        )))
        .stderr(predicates::str::is_empty());

    let document = run_json(&repo, &["show", "--format", "json", "--mode", "signatures"]);
    assert_eq!(document["schema"], "codect.show.v1");
    let file = &document["files"][0];
    assert_eq!(file["path"], "src/Point.swift");
    assert_eq!(
        file["language"], "swift",
        "JSON must label the Swift language"
    );
    assert_eq!(projection_text(file), POINT_SIGNATURES);
    validate_show(&document);
}

#[test]
fn swift_explicit_revision_stdin_and_worktree_agree() {
    let repo = TestRepo::init();
    repo.write("src/Point.swift", POINT);
    repo.commit("base");

    let revision = run_json(
        &repo,
        &["show", "--format", "json", "--mode", "types", "HEAD"],
    );
    assert_eq!(revision["input"], "revision");
    assert_eq!(revision["revision"], "HEAD");

    let stdin = show_json_stdin(&repo, "src/Point.swift", "types", POINT);
    let worktree = run_json(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--worktree",
            "--path",
            "src/Point.swift",
            "--mode",
            "types",
        ],
    );
    assert_eq!(stdin["input"], "stdin");
    assert_eq!(worktree["input"], "worktree");

    for document in [&revision, &stdin, &worktree] {
        let file = &document["files"][0];
        assert_eq!(file["language"], "swift");
        assert_eq!(projection_text(file), POINT_TYPES);
        assert_eq!(file["outline"], revision["files"][0]["outline"]);
        validate_show(document);
    }

    // Text mode at an explicit revision carries the same framing as disk show.
    let text = stdout(&run(&repo, &["show", "--mode", "signatures", "HEAD"]));
    assert_eq!(text, format!("== src/Point.swift ==\n{POINT_SIGNATURES}"));
}

#[test]
fn swift_tests_mode_is_available_on_stdin_disk_and_json() {
    let repo = TestRepo::init();
    repo.write("src/Tests.swift", SWIFT_TESTS);
    repo.commit("base");

    let text = stdout(&run(&repo, &["show", "--mode", "tests"]));
    assert_eq!(text, format!("== src/Tests.swift ==\n{SWIFT_TESTS_GOLDEN}"));

    let stdin = show_json_stdin(&repo, "src/Tests.swift", "tests", SWIFT_TESTS);
    assert_eq!(file_at(&stdin, "src/Tests.swift")["language"], "swift");
    assert_eq!(
        projection_text(file_at(&stdin, "src/Tests.swift")),
        SWIFT_TESTS_GOLDEN
    );
    validate_show(&stdin);
}

#[test]
fn swift_snapshots_index_worktree_and_empty_validate_against_diff_schema() {
    // `:index`: a staged signature change is a focused modification.
    let staged = TestRepo::init();
    let changed = POINT.replace("by dx: Double", "by dx: Int");
    staged.write("src/Point.swift", POINT);
    staged.commit("base");
    staged.write("src/Point.swift", &changed);
    staged.git_ok(&["add", "src/Point.swift"]);
    let document = run_json(
        &staged,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "signatures",
            "HEAD",
            ":index",
        ],
    );
    assert_eq!(document["schema"], "codect.diff.v2");
    assert_eq!(statuses(&document), ["modified"]);
    assert_eq!(file_at(&document, "src/Point.swift")["language"], "swift");
    validate_diff(&document);

    // `:worktree`: an untracked Swift file is a worktree member and appears as
    // an addition against the index.
    staged.write("src/Fresh.swift", POINT);
    let document = run_json(
        &staged,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "types",
            ":index",
            ":worktree",
        ],
    );
    assert_eq!(paths(&document), ["src/Fresh.swift"]);
    assert_eq!(file_at(&document, "src/Fresh.swift")["status"], "added");
    validate_diff(&document);

    // `:empty`: no false moves on an added-only or deleted-only side.
    let document = run_json(
        &staged,
        &[
            "diff", "--format", "json", "--mode", "types", ":empty", "HEAD",
        ],
    );
    assert_eq!(statuses(&document), ["added"]);
    assert!(!statuses(&document).contains(&"moved"));
    validate_diff(&document);

    let document = run_json(
        &staged,
        &[
            "diff", "--format", "json", "--mode", "types", "HEAD", ":empty",
        ],
    );
    assert_eq!(statuses(&document), ["deleted"]);
    validate_diff(&document);
}

#[test]
fn swift_path_and_area_scoping_emit_only_selected_files() {
    let repo = TestRepo::init();
    repo.write("apps/ui/Widget.swift", POINT);
    repo.write("services/api/Model.swift", POINT);
    repo.write(
        ".codect.toml",
        "[areas]\nui = [\"apps/ui\"]\napi = [\"services/api\"]\n",
    );
    repo.commit("base");

    let by_path = run_json(
        &repo,
        &[
            "show", "--format", "json", "--mode", "types", "--path", "apps/ui",
        ],
    );
    assert_eq!(paths(&by_path), ["apps/ui/Widget.swift"]);
    validate_show(&by_path);

    let by_area = run_json(
        &repo,
        &[
            "show", "--format", "json", "--mode", "types", "--area", "api", "--area", "ui",
        ],
    );
    assert_eq!(
        paths(&by_area),
        ["apps/ui/Widget.swift", "services/api/Model.swift"]
    );
    validate_show(&by_area);
}

#[test]
fn swift_body_only_change_is_empty_and_signature_change_is_not() {
    let repo = TestRepo::init();
    let body_variant = POINT.replace("y: y)", "y: y + 0)");
    repo.write("src/Point.swift", POINT);
    repo.commit("base");
    repo.write("src/Point.swift", &body_variant);
    repo.commit("body only");

    for mode in ["types", "signatures"] {
        codect_in(&repo, &["diff", "--mode", mode, "HEAD~1", "HEAD"])
            .assert()
            .success()
            .stdout(predicates::str::is_empty())
            .stderr(predicates::str::is_empty());
    }
    let document = run_json(
        &repo,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "signatures",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert!(files(&document).is_empty(), "body-only diff has no events");
    validate_diff(&document);

    // A parameter type edit changes only Signatures: `shifted` is a method,
    // which Types drops.
    repo.write(
        "src/Point.swift",
        &POINT.replace("by dx: Double", "by dx: Int"),
    );
    repo.commit("signature change");
    codect_in(&repo, &["diff", "--mode", "types", "HEAD~1", "HEAD"])
        .assert()
        .success()
        .stdout(predicates::str::is_empty());
    let signatures = stdout(&run(
        &repo,
        &["diff", "--mode", "signatures", "HEAD~1", "HEAD"],
    ));
    assert!(
        signatures.contains("-    func shifted(by dx: Double) -> Point { ... }")
            && signatures.contains("+    func shifted(by dx: Int) -> Point { ... }"),
        "signature change missing: {signatures}"
    );
}

#[test]
fn swift_exact_body_only_and_fuzzy_moves_are_visible_in_text_and_json() {
    // Exact move: pure rename with no focused hunks.
    let exact = TestRepo::init();
    exact.write("src/old.swift", POINT);
    exact.commit("base");
    exact.git_ok(&["mv", "src/old.swift", "src/new.swift"]);
    exact.commit("move");

    let text = stdout(&run(&exact, &["diff", "--mode", "types", "HEAD~1", "HEAD"]));
    assert!(text.contains("rename from src/old.swift\n"), "{text}");
    assert!(text.contains("rename to src/new.swift\n"), "{text}");
    assert!(!text.contains("@@"), "an exact move has no hunks: {text}");

    let document = run_json(
        &exact,
        &[
            "diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD",
        ],
    );
    assert_eq!(statuses(&document), ["moved"]);
    let moved = file_at(&document, "src/new.swift");
    assert_eq!(moved["old_path"], "src/old.swift");
    assert_eq!(moved["equal"], true);
    assert_eq!(moved["language"], "swift");
    validate_diff(&document);

    // Body-only edit inside a move still pairs and stays equal.
    let body = TestRepo::init();
    body.write("src/old.swift", POINT);
    body.commit("base");
    body.git_ok(&["mv", "src/old.swift", "src/new.swift"]);
    body.write("src/new.swift", &POINT.replace("y: y)", "y: y + 0)"));
    body.commit("body-only move");
    let document = run_json(
        &body,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "signatures",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert_eq!(file_at(&document, "src/new.swift")["equal"], true);
    validate_diff(&document);

    // Edited rename: high source similarity, so fuzzy pairing still finds it,
    // but the focused projections differ.
    let fuzzy = TestRepo::init();
    let old = big_source(false);
    let new = big_source(true);
    fuzzy.write("src/old.swift", &old);
    fuzzy.commit("base");
    fuzzy.remove("src/old.swift");
    fuzzy.write("src/new.swift", &new);
    fuzzy.commit("edited move");
    let document = run_json(
        &fuzzy,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "signatures",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert_eq!(statuses(&document), ["moved"]);
    let moved = file_at(&document, "src/new.swift");
    assert_eq!(moved["old_path"], "src/old.swift");
    assert_eq!(moved["equal"], false);
    validate_diff(&document);
}

fn big_source(edited: bool) -> String {
    let mut text = String::from("struct Big {\n");
    for index in 0..40 {
        let ty = if edited && index == 7 { "Int64" } else { "Int" };
        text.push_str(&format!("    let f{index:02}: {ty}\n"));
    }
    text.push_str("}\n");
    text
}

#[test]
fn swift_atomic_failure_keeps_stdout_empty_with_a_valid_file_selected() {
    let repo = TestRepo::init();
    // `aaa` sorts before `broken`, so a partial document is tempting.
    repo.write("src/aaa.swift", POINT);
    repo.write("src/broken.swift", BROKEN_SWIFT);
    repo.commit("broken");

    for args in [
        vec!["show", "--mode", "types", "HEAD"],
        vec!["show", "--format", "json", "--mode", "signatures", "HEAD"],
    ] {
        let output = run(&repo, &args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "no partial document: {args:?}");
        assert!(
            stderr(&output).contains("src/broken.swift"),
            "diagnostic must name the failing file: {}",
            stderr(&output)
        );
    }

    // The same atomicity holds for a diff: the valid file is projected first,
    // but no output is written once the invalid one fails.
    repo.remove("src/broken.swift");
    repo.commit("valid target");
    let output = run(
        &repo,
        &[
            "diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "no partial diff: {:?}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("src/broken.swift"),
        "diagnostic: {}",
        stderr(&output)
    );
}

#[test]
fn swift_tests_mode_show_and_diff_follow_the_signatures_filter() {
    let repo = TestRepo::init();
    repo.write("src/Tests.swift", SWIFT_TESTS);
    repo.commit("base");

    let shown = stdout(&run(&repo, &["show", "--mode", "tests"]));
    assert_eq!(
        shown,
        format!("== src/Tests.swift ==\n{SWIFT_TESTS_GOLDEN}")
    );

    let renamed = SWIFT_TESTS.replace("testThing", "testRenamed");
    repo.write("src/Tests.swift", &renamed);
    repo.commit("rename test");
    let diffed = stdout(&run(&repo, &["diff", "--mode", "tests", "HEAD~1", "HEAD"]));
    assert!(
        diffed.contains("-    func testThing() { ... }")
            && diffed.contains("+    func testRenamed() { ... }"),
        "test rename missing: {diffed}"
    );

    let document = run_json(
        &repo,
        &[
            "diff", "--format", "json", "--mode", "tests", "HEAD~1", "HEAD",
        ],
    );
    assert_eq!(statuses(&document), ["modified"]);
    validate_diff(&document);
}
