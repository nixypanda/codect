//! End-to-end tests for the `ownai.show.v1` JSON document
//! (`ownai show --format json`).
//!
//! These cover both input paths: a committed revision and editor-supplied
//! bytes (stdin and worktree). They assert the document contract — identical
//! `stable_key` values and outline structure between the two input paths, the
//! mode-independent outline, byte-order file ordering, usage errors, and the
//! absence of ANSI — and validate the document against
//! `docs/schema/ownai.show.v1.json`.

mod support;

use serde_json::Value;
use support::{TestRepo, doc, fixture, ownai_in, stderr, stdout};

const RUST: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

fn schema() -> Value {
    serde_json::from_str(&doc("schema/ownai.show.v1.json")).expect("the schema is valid JSON")
}

/// An outline with `retained_in_mode` removed, for mode-independent comparison.
fn outline_shape(outline: &Value) -> Vec<Value> {
    outline
        .as_array()
        .expect("outline array")
        .iter()
        .map(|item| {
            let mut item = item.clone();
            item.as_object_mut()
                .expect("outline item")
                .remove("retained_in_mode");
            item
        })
        .collect()
}

fn parse(output: &std::process::Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("stdout is a valid JSON document")
}

/// Projects `source` through stdin with `path` as the language/path context.
fn show_json_stdin(repo: &TestRepo, path: &str, mode: &str, source: &str) -> Value {
    let output = ownai_in(
        repo,
        &[
            "show", "--format", "json", "--stdin", "--path", path, "--mode", mode,
        ],
    )
    .write_stdin(source)
    .output()
    .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    parse(&output)
}

/// Projects `path` from the worktree.
fn show_json_worktree(repo: &TestRepo, path: &str, mode: &str) -> Value {
    let output = ownai_in(
        repo,
        &[
            "show",
            "--format",
            "json",
            "--worktree",
            "--path",
            path,
            "--mode",
            mode,
        ],
    )
    .output()
    .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    parse(&output)
}

/// Projects `path` from the committed `HEAD`.
fn show_json_revision(repo: &TestRepo, path: &str, mode: &str) -> Value {
    let output = ownai_in(
        repo,
        &["show", "--format", "json", "--path", path, "--mode", mode],
    )
    .output()
    .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    parse(&output)
}

#[test]
fn stdin_and_worktree_documents_match_modulo_input() {
    let repo = TestRepo::init();
    let source = fixture("rust/implementations/input.rs");
    repo.write("src/lib.rs", &source);
    repo.commit("base");

    let stdin_doc = show_json_stdin(&repo, "src/lib.rs", "types", &source);
    let worktree_doc = show_json_worktree(&repo, "src/lib.rs", "types");

    assert_eq!(stdin_doc["schema"], "ownai.show.v1");
    assert_eq!(stdin_doc["input"], "stdin");
    assert_eq!(worktree_doc["input"], "worktree");
    assert_eq!(stdin_doc["revision"], Value::Null);
    assert_eq!(worktree_doc["revision"], Value::Null);

    // The only difference between the two input paths is the `input` label.
    let mut normalized = stdin_doc.clone();
    normalized["input"] = Value::String("worktree".to_owned());
    assert_eq!(normalized, worktree_doc);
}

#[test]
fn stdin_and_revision_agree_on_projection_outline_and_stable_keys() {
    let repo = TestRepo::init();
    let source = fixture("rust/implementations/input.rs");
    repo.write("src/lib.rs", &source);
    repo.commit("base");

    for mode in ["types", "signatures"] {
        let stdin_doc = show_json_stdin(&repo, "src/lib.rs", mode, &source);
        let revision_doc = show_json_revision(&repo, "src/lib.rs", mode);

        assert_eq!(revision_doc["input"], "revision");
        assert_eq!(revision_doc["revision"], "HEAD");

        let from_stdin = &stdin_doc["files"][0];
        let from_revision = &revision_doc["files"][0];

        assert_eq!(
            from_stdin["projection"]["text"], from_revision["projection"]["text"],
            "{mode}: projection text differs"
        );
        assert_eq!(
            from_stdin["projection"]["items"], from_revision["projection"]["items"],
            "{mode}: projection items differ"
        );
        assert_eq!(
            from_stdin["outline"], from_revision["outline"],
            "{mode}: outline differs"
        );
    }
}

#[test]
fn outline_is_mode_independent_and_marks_dropped_declarations() {
    let repo = TestRepo::init();
    let source = fixture("rust/implementations/input.rs");
    repo.write("src/lib.rs", &source);
    repo.commit("base");

    let types = show_json_stdin(&repo, "src/lib.rs", "types", &source);
    let signatures = show_json_stdin(&repo, "src/lib.rs", "signatures", &source);

    let types_file = &types["files"][0];
    let signatures_file = &signatures["files"][0];

    // The outline is complete in both modes and mode-independent: the same
    // declarations with the same spans and signatures, differing only in
    // `retained_in_mode`.
    assert_eq!(
        outline_shape(&types_file["outline"]),
        outline_shape(&signatures_file["outline"])
    );

    let outline = types_file["outline"].as_array().expect("outline array");
    assert!(
        outline
            .iter()
            .any(|item| item["retained_in_mode"] == Value::Bool(false)),
        "Types mode must drop some declarations: {outline:?}"
    );

    // A declaration dropped by Types mode is still located by the outline, and
    // is absent from the Types projection.
    let projected_keys: Vec<&str> = types_file["projection"]["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|item| item["stable_key"].as_str().expect("stable_key"))
        .collect();
    for item in outline {
        let key = item["stable_key"].as_str().expect("stable_key");
        assert_eq!(
            item["retained_in_mode"] == Value::Bool(true),
            projected_keys.contains(&key),
            "retained_in_mode disagrees with the Types projection for {key}"
        );
    }
}

#[test]
fn json_path_and_area_scoping_emit_files_in_raw_byte_order() {
    let repo = TestRepo::init();
    repo.write(
        "apps/web/src/App.elm",
        &fixture("elm/type-aliases/input.elm"),
    );
    repo.write(
        "libs/ui/src/Widget.elm",
        &fixture("elm/normal-module/input.elm"),
    );
    repo.write("services/api/src/lib.rs", &fixture("rust/structs/input.rs"));
    repo.write(
        ".ownai.toml",
        "[areas]\nfrontend = [\"apps/web\", \"libs/ui\"]\nbackend = [\"services/api\"]\n",
    );
    repo.commit("base");

    let by_area = ownai_in(
        &repo,
        &[
            "show", "--format", "json", "--area", "frontend", "--area", "backend", "--mode",
            "types",
        ],
    )
    .output()
    .expect("run ownai");
    assert!(by_area.status.success(), "stderr: {}", stderr(&by_area));
    let area_doc = parse(&by_area);
    let area_paths: Vec<&str> = area_doc["files"]
        .as_array()
        .expect("files array")
        .iter()
        .map(|file| file["path"].as_str().expect("path"))
        .collect();
    assert_eq!(
        area_paths,
        [
            "apps/web/src/App.elm",
            "libs/ui/src/Widget.elm",
            "services/api/src/lib.rs"
        ]
    );

    let by_path = ownai_in(
        &repo,
        &[
            "show", "--format", "json", "--path", "services", "--path", "libs", "--mode", "types",
        ],
    )
    .output()
    .expect("run ownai");
    assert!(by_path.status.success(), "stderr: {}", stderr(&by_path));
    let path_doc = parse(&by_path);
    let path_paths: Vec<&str> = path_doc["files"]
        .as_array()
        .expect("files array")
        .iter()
        .map(|file| file["path"].as_str().expect("path"))
        .collect();
    assert_eq!(
        path_paths,
        ["libs/ui/src/Widget.elm", "services/api/src/lib.rs"]
    );
}

#[test]
fn stdin_usage_errors_exit_two() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST);
    repo.commit("base");

    // `--stdin` without `--path`.
    ownai_in(
        &repo,
        &["show", "--format", "json", "--mode", "types", "--stdin"],
    )
    .write_stdin(RUST)
    .assert()
    .code(2);

    // `--stdin` with a revision.
    ownai_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--mode",
            "types",
            "--stdin",
            "--path",
            "src/lib.rs",
            "HEAD",
        ],
    )
    .write_stdin(RUST)
    .assert()
    .code(2);

    // `--stdin` with `--area`.
    ownai_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--mode",
            "types",
            "--stdin",
            "--path",
            "src/lib.rs",
            "--area",
            "frontend",
        ],
    )
    .write_stdin(RUST)
    .assert()
    .code(2);

    // More than one `--path` is not one language/path context.
    ownai_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--mode",
            "types",
            "--stdin",
            "--path",
            "src/lib.rs",
            "--path",
            "src/lib.rs",
        ],
    )
    .write_stdin(RUST)
    .assert()
    .code(2);
}

#[test]
fn source_failures_exit_one_with_empty_stdout() {
    let repo = TestRepo::init();
    repo.write("README.md", "not a supported source file\n");
    repo.write("src/lib.rs", RUST);
    repo.commit("base");

    // Unsupported extension.
    let unsupported = ownai_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--stdin",
            "--path",
            "README.md",
            "--mode",
            "types",
        ],
    )
    .write_stdin("not a supported source file\n")
    .output()
    .expect("run ownai");
    assert_eq!(unsupported.status.code(), Some(1));
    assert!(unsupported.stdout.is_empty(), "stdout must stay empty");

    // A path outside the repository.
    let outside = ownai_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--stdin",
            "--path",
            "/etc/passwd",
            "--mode",
            "types",
        ],
    )
    .write_stdin("x\n")
    .output()
    .expect("run ownai");
    assert_eq!(outside.status.code(), Some(1));
    assert!(outside.stdout.is_empty(), "stdout must stay empty");

    // Non-UTF-8 source bytes.
    let invalid = ownai_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--stdin",
            "--path",
            "src/lib.rs",
            "--mode",
            "types",
        ],
    )
    .write_stdin(b"pub fn main() { \xFF }\n".to_vec())
    .output()
    .expect("run ownai");
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&invalid).contains("UTF-8"),
        "diagnostic: {}",
        stderr(&invalid)
    );
}

#[test]
fn json_output_contains_no_ansi_even_with_color_always() {
    let repo = TestRepo::init();
    let source = fixture("rust/structs/input.rs");
    repo.write("src/lib.rs", &source);
    repo.commit("base");

    let output = ownai_in(
        &repo,
        &[
            "--color=always",
            "show",
            "--format",
            "json",
            "--stdin",
            "--path",
            "src/lib.rs",
            "--mode",
            "types",
        ],
    )
    .write_stdin(source.as_str())
    .output()
    .expect("run ownai");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains('\u{1b}'),
        "JSON must never carry ANSI"
    );
}

#[test]
fn golden_documents_match_and_validate_against_the_schema() {
    let schema = schema();
    let source = fixture("schema/input.rs");
    // `--stdin` needs a discoverable repository but no commit; the path is
    // resolved lexically and need not exist on disk.
    let repo = TestRepo::init();

    for (mode, golden) in [
        ("types", "schema/show-types.json"),
        ("signatures", "schema/show-signatures.json"),
    ] {
        let output = ownai_in(
            &repo,
            &[
                "show",
                "--format",
                "json",
                "--stdin",
                "--path",
                "fixtures/schema/input.rs",
                "--mode",
                mode,
            ],
        )
        .write_stdin(source.as_str())
        .output()
        .expect("run ownai");
        assert!(output.status.success(), "stderr: {}", stderr(&output));

        let text = stdout(&output);
        assert_eq!(text, fixture(golden), "{mode} golden document mismatch");

        let value = parse(&output);
        support::schema::validate(&schema, &value)
            .unwrap_or_else(|error| panic!("{mode} document is not schema-valid: {error}"));
    }
}

#[test]
fn all_four_languages_emit_a_schema_valid_outline() {
    let schema = schema();
    let repo = TestRepo::init();

    for (fixture_path, language) in [
        ("elm/canonical-signatures/input.elm", "elm"),
        ("haskell/classes-instances/input.hs", "haskell"),
        ("python/classes-and-fields/input.py", "python"),
        ("rust/implementations/input.rs", "rust"),
    ] {
        let source = fixture(fixture_path);
        let file_name = fixture_path.rsplit('/').next().expect("file name");
        let path = format!("src/{file_name}");

        let document = show_json_stdin(&repo, &path, "types", &source);
        let file = &document["files"][0];
        assert_eq!(file["language"], language, "{fixture_path}");
        assert!(
            !file["outline"]
                .as_array()
                .expect("outline array")
                .is_empty(),
            "{fixture_path}: outline must not be empty"
        );
        support::schema::validate(&schema, &document)
            .unwrap_or_else(|error| panic!("{fixture_path} document is not schema-valid: {error}"));
    }
}
