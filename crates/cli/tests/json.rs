// End-to-end tests for the `codect.show.v1` JSON document
// (`codect show --format json`).
//
// These cover both input paths: a committed revision and editor-supplied
// bytes (stdin and worktree). They assert the document contract — identical
// `stable_key` values and outline structure between the two input paths, the
// mode-independent outline, byte-order file ordering, usage errors, and the
// absence of ANSI — and validate the document against
// `docs/schema/codect.show.v1.json`.

mod support;

use serde_json::Value;
use support::{TestRepo, codect_in, doc, fixture, stderr, stdout};

const RUST: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

fn schema() -> Value {
    serde_json::from_str(&doc("schema/codect.show.v1.json")).expect("the schema is valid JSON")
}

fn outline_keys(outline: &Value) -> Vec<&str> {
    outline
        .as_array()
        .expect("outline array")
        .iter()
        .map(|item| item["stable_key"].as_str().expect("stable_key"))
        .collect()
}

fn line_number(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no source line contains {needle:?}"))
        + 1
}

fn outline_item<'a>(file: &'a Value, key: &str) -> &'a Value {
    file["outline"]
        .as_array()
        .expect("outline array")
        .iter()
        .find(|item| item["stable_key"] == key)
        .unwrap_or_else(|| panic!("no outline item with stable_key {key}"))
}

fn projection_item<'a>(file: &'a Value, key: &str) -> Option<&'a Value> {
    file["projection"]["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|item| item["stable_key"] == key)
}

fn parse(output: &std::process::Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("stdout is a valid JSON document")
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
    parse(&output)
}

fn show_json_worktree(repo: &TestRepo, path: &str, mode: &str) -> Value {
    let output = codect_in(
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
    .expect("run codect");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    parse(&output)
}

fn show_json_revision(repo: &TestRepo, path: &str, mode: &str) -> Value {
    let output = codect_in(
        repo,
        &["show", "--format", "json", "--path", path, "--mode", mode],
    )
    .output()
    .expect("run codect");
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

    assert_eq!(stdin_doc["schema"], "codect.show.v1");
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

    // The Types-mode outline enumerates the Signatures projection (the
    // superset), not the Types projection it is paired with. This crosses two
    // genuinely different projections, so it is not true by construction of a
    // single document.
    assert_eq!(
        outline_keys(&types_file["outline"]),
        outline_keys(&signatures_file["projection"]["items"]),
        "the Types-mode outline must enumerate the Signatures projection"
    );

    let outline = types_file["outline"].as_array().expect("outline array");
    assert!(
        outline
            .iter()
            .any(|item| item["retained_in_mode"] == Value::Bool(false)),
        "Types mode must drop some declarations: {outline:?}"
    );

    // `retained_in_mode` is exactly membership in the requested projection: a
    // declaration dropped by Types mode is still located by the outline and is
    // absent from the Types projection items.
    let types_projection = &types_file["projection"]["items"];
    for item in outline {
        let key = item["stable_key"].as_str().expect("stable_key");
        assert_eq!(
            item["retained_in_mode"] == Value::Bool(true),
            projection_item(types_file, key).is_some(),
            "retained_in_mode disagrees with the Types projection for {key}"
        );
    }
    assert!(
        !types_projection.as_array().expect("items array").is_empty(),
        "Types mode keeps the trait implementations that carry types"
    );
}

#[test]
fn retained_outline_text_comes_from_the_projection_and_superset_signature_is_the_fallback() {
    let repo = TestRepo::init();
    let source = fixture("rust/implementations/input.rs");
    repo.write("src/lib.rs", &source);
    repo.commit("base");

    let types = show_json_stdin(&repo, "src/lib.rs", "types", &source);
    let file = &types["files"][0];

    // A *container* the requested mode retains still differs from its outline
    // signature: Types keeps the trait implementation with only its associated
    // type, while the Signatures superset also shows the method. The mode-correct
    // closed-fold text is therefore the projection item, not `signature`.
    let container = "impl Iterator for Users<T>";
    let container_item = outline_item(file, container);
    assert_eq!(
        container_item["retained_in_mode"],
        Value::Bool(true),
        "{container}"
    );
    let projection = projection_item(file, container).expect("Types retains the container");
    assert_ne!(
        container_item["signature"], projection["canonical_text"],
        "{container}: the superset signature must differ from the Types projection"
    );
    assert!(
        container_item["signature"]
            .as_str()
            .expect("signature")
            .contains("fn next"),
        "the superset signature shows the Signatures member"
    );
    assert!(
        !projection["canonical_text"]
            .as_str()
            .expect("canonical_text")
            .contains("fn next"),
        "the Types projection omits the Signatures member"
    );

    // A declaration the mode drops has no projection item; `signature` is the
    // only closed-fold text available and is the superset form.
    let dropped = "impl User::method::id";
    let dropped_item = outline_item(file, dropped);
    assert_eq!(
        dropped_item["retained_in_mode"],
        Value::Bool(false),
        "{dropped}"
    );
    assert!(
        projection_item(file, dropped).is_none(),
        "{dropped} must not appear in the Types projection"
    );
    assert!(
        dropped_item["signature"]
            .as_str()
            .expect("signature")
            .contains("fn id"),
        "{dropped}: the fallback signature still renders the dropped declaration"
    );

    // For a non-container declaration the two coincide, which is why the rule
    // only matters for containers.
    let simple = "impl Clone for User";
    let simple_item = outline_item(file, simple);
    let simple_projection = projection_item(file, simple).expect("Types retains the impl");
    assert_eq!(
        simple_item["signature"], simple_projection["canonical_text"],
        "{simple}: a non-container's outline signature equals its projection text"
    );
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
        ".codect.toml",
        "[areas]\nfrontend = [\"apps/web\", \"libs/ui\"]\nbackend = [\"services/api\"]\n",
    );
    repo.commit("base");

    let by_area = codect_in(
        &repo,
        &[
            "show", "--format", "json", "--area", "frontend", "--area", "backend", "--mode",
            "types",
        ],
    )
    .output()
    .expect("run codect");
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

    let by_path = codect_in(
        &repo,
        &[
            "show", "--format", "json", "--path", "services", "--path", "libs", "--mode", "types",
        ],
    )
    .output()
    .expect("run codect");
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

    codect_in(
        &repo,
        &["show", "--format", "json", "--mode", "types", "--stdin"],
    )
    .write_stdin(RUST)
    .assert()
    .code(2);

    codect_in(
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

    codect_in(
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
    codect_in(
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
fn source_path_that_names_a_directory_is_a_usage_error() {
    let repo = TestRepo::init();
    // Create a real non-root directory so the check is about the filesystem
    // shape, not a missing path.
    repo.write("crates/inner/lib.rs", RUST);
    repo.write("src/lib.rs", RUST);
    repo.commit("base");

    for args in [
        vec![
            "show", "--format", "json", "--mode", "types", "--stdin", "--path", "crates",
        ],
        vec![
            "show",
            "--format",
            "json",
            "--mode",
            "types",
            "--worktree",
            "--path",
            "crates",
        ],
    ] {
        let output = codect_in(&repo, &args)
            .write_stdin(RUST)
            .output()
            .expect("run codect");
        assert_eq!(
            output.status.code(),
            Some(2),
            "directory `--path` must be a usage error: {args:?}"
        );
        assert!(
            output.stdout.is_empty(),
            "usage error must not write stdout: {args:?}"
        );
    }

    let root = codect_in(
        &repo,
        &[
            "show", "--format", "json", "--mode", "types", "--stdin", "--path", ".",
        ],
    )
    .write_stdin(RUST)
    .output()
    .expect("run codect");
    assert_eq!(root.status.code(), Some(2));
    assert!(root.stdout.is_empty(), "usage error must not write stdout");
}

#[cfg(unix)]
#[test]
fn worktree_symlink_that_leaves_the_repository_is_rejected() {
    use std::os::unix::fs::symlink;

    let repo = TestRepo::init();
    // The symlink target lives in a different temporary directory, so it is
    // unambiguously outside the repository.
    let outside = TestRepo::new();
    outside.write("target.rs", "pub struct Leaked {}\n");

    repo.write("src/lib.rs", RUST);
    repo.commit("base");
    symlink(
        outside.path().join("target.rs"),
        repo.path().join("src/leak.rs"),
    )
    .expect("create in-repo symlink");

    let output = codect_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--mode",
            "types",
            "--worktree",
            "--path",
            "src/leak.rs",
        ],
    )
    .output()
    .expect("run codect");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("symlink"),
        "diagnostic: {}",
        stderr(&output)
    );

    // A symlinked directory that escapes the repository is caught by the
    // resolved-path check even though the final component is a regular file.
    symlink(outside.path(), repo.path().join("linked")).expect("create directory symlink");
    let escaped = codect_in(
        &repo,
        &[
            "show",
            "--format",
            "json",
            "--mode",
            "types",
            "--worktree",
            "--path",
            "linked/target.rs",
        ],
    )
    .output()
    .expect("run codect");
    assert_eq!(escaped.status.code(), Some(1));
    assert!(escaped.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&escaped).contains("outside the repository"),
        "diagnostic: {}",
        stderr(&escaped)
    );
}

#[test]
fn source_failures_exit_one_with_empty_stdout() {
    let repo = TestRepo::init();
    repo.write("README.md", "not a supported source file\n");
    repo.write("src/lib.rs", RUST);
    repo.commit("base");

    let unsupported = codect_in(
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
    .expect("run codect");
    assert_eq!(unsupported.status.code(), Some(1));
    assert!(unsupported.stdout.is_empty(), "stdout must stay empty");

    let outside = codect_in(
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
    .expect("run codect");
    assert_eq!(outside.status.code(), Some(1));
    assert!(outside.stdout.is_empty(), "stdout must stay empty");

    let invalid = codect_in(
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
    .expect("run codect");
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

    let output = codect_in(
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
    .expect("run codect");
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
        ("tests", "schema/show-tests.json"),
    ] {
        let output = codect_in(
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
        .expect("run codect");
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

#[test]
fn span_covers_the_full_declaration_body_for_rust_and_python() {
    let repo = TestRepo::init();

    // Rust: a trait implementation from its header line to its closing brace.
    let rust_source = fixture("rust/implementations/input.rs");
    let rust = show_json_stdin(&repo, "src/lib.rs", "types", &rust_source);
    let rust_file = &rust["files"][0];
    let container = outline_item(rust_file, "impl Iterator for Users<T>");
    let start = line_number(&rust_source, "impl<T: Clone> Iterator for Users<T>");
    assert_eq!(container["span"]["start_line"], start, "Rust start line");
    assert!(
        rust_source
            .lines()
            .nth(start - 1)
            .expect("start line")
            .contains("impl<T: Clone> Iterator for Users<T>"),
        "the span must start at the declaration node"
    );
    let end = container["span"]["end_line"].as_u64().expect("end_line") as usize;
    assert_eq!(
        rust_source.lines().nth(end - 1).expect("end line").trim(),
        "}",
        "the Rust span must end at the closing brace"
    );

    // Python: a function from its `def` line to the last line of its suite.
    let python_source = fixture("python/classes-and-fields/input.py");
    let python = show_json_stdin(&repo, "src/app.py", "signatures", &python_source);
    let python_file = &python["files"][0];
    let area = outline_item(python_file, "src/app.py::fn::area");
    assert_eq!(
        area["span"]["start_line"],
        line_number(&python_source, "def area("),
        "Python start line"
    );
    let end = area["span"]["end_line"].as_u64().expect("end_line") as usize;
    assert_eq!(
        python_source.lines().nth(end - 1).expect("end line").trim(),
        "return width * height",
        "the Python span must end at the last body line"
    );

    // A leading decorator is outside the span: the class starts on its `class`
    // line, not the decorator line, even though `signature` includes it.
    let point = outline_item(python_file, "src/app.py::class::Point");
    assert_eq!(
        point["span"]["start_line"],
        line_number(&python_source, "class Point("),
        "the decorator line must not be the span start"
    );
    assert!(
        point["signature"]
            .as_str()
            .expect("signature")
            .contains("@dataclass"),
        "the signature still carries the decorator"
    );
}

#[test]
fn nested_items_carry_parent_keys_for_elm_haskell_and_python() {
    let repo = TestRepo::init();

    // Elm: a constructor's parent is its custom type.
    let elm_source = fixture("elm/constructor-arity/input.elm");
    let elm = show_json_stdin(&repo, "src/App.elm", "types", &elm_source);
    let elm_file = &elm["files"][0];
    let constructor = outline_item(elm_file, "Shapes constructor Unit");
    assert_eq!(constructor["kind"], "constructor");
    assert_eq!(constructor["parent_key"], "Shapes type Shape");
    assert_eq!(
        outline_item(elm_file, "Shapes type Shape")["parent_key"],
        Value::Null,
        "a top-level type has no parent"
    );

    // Haskell: a class method's parent is the class, and it is outlined even
    // though Types mode drops it.
    let haskell_source = fixture("haskell/classes-instances/input.hs");
    let haskell = show_json_stdin(&repo, "src/Main.hs", "types", &haskell_source);
    let haskell_file = &haskell["files"][0];
    let method = outline_item(haskell_file, "src/Main.hs::class::Container::method::empty");
    assert_eq!(method["kind"], "method");
    assert_eq!(method["parent_key"], "src/Main.hs::class::Container");
    assert_eq!(method["retained_in_mode"], Value::Bool(false));

    // Python: a class field's parent is the class.
    let python_source = fixture("python/classes-and-fields/input.py");
    let python = show_json_stdin(&repo, "src/app.py", "types", &python_source);
    let python_file = &python["files"][0];
    let field = outline_item(python_file, "src/app.py::class::Point::field::x");
    assert_eq!(field["kind"], "field");
    assert_eq!(field["parent_key"], "src/app.py::class::Point");
    assert_eq!(
        outline_item(python_file, "src/app.py::class::Point")["parent_key"],
        Value::Null
    );
}

#[test]
fn revision_documents_validate_against_the_schema() {
    let schema = schema();
    let repo = TestRepo::init();
    repo.write("src/App.elm", &fixture("elm/type-aliases/input.elm"));
    repo.write("src/Main.hs", &fixture("haskell/canonical-types/input.hs"));
    repo.write("src/app.py", &fixture("python/canonical-types/input.py"));
    repo.write("src/lib.rs", &fixture("rust/structs/input.rs"));
    repo.commit("base");

    for mode in ["types", "signatures"] {
        let document = show_json_revision(&repo, "src/lib.rs", mode);
        assert_eq!(document["input"], "revision");
        assert_eq!(document["revision"], "HEAD");
        support::schema::validate(&schema, &document).unwrap_or_else(|error| {
            panic!("{mode} revision document is not schema-valid: {error}")
        });
    }

    // The whole multi-file document validates too, in raw path byte order.
    let output = codect_in(&repo, &["show", "--format", "json", "--mode", "types"])
        .output()
        .expect("run codect");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let document = parse(&output);
    let paths: Vec<&str> = document["files"]
        .as_array()
        .expect("files array")
        .iter()
        .map(|file| file["path"].as_str().expect("path"))
        .collect();
    assert_eq!(
        paths,
        ["src/App.elm", "src/Main.hs", "src/app.py", "src/lib.rs"]
    );
    support::schema::validate(&schema, &document)
        .unwrap_or_else(|error| panic!("revision document is not schema-valid: {error}"));
}

#[test]
fn unknown_fields_are_tolerated_and_required_fields_stay_strict() {
    let schema = schema();
    let repo = TestRepo::init();
    let source = fixture("rust/structs/input.rs");

    let mut document = show_json_stdin(&repo, "src/lib.rs", "types", &source);
    // Simulate a consumer reading a newer v1 document: unknown fields may
    // appear at every level and must not invalidate the document.
    document
        .as_object_mut()
        .expect("document object")
        .insert("future_top_level".to_owned(), Value::from(1));
    document["files"][0]
        .as_object_mut()
        .expect("file object")
        .insert("future_file".to_owned(), Value::Null);
    document["files"][0]["projection"]
        .as_object_mut()
        .expect("projection object")
        .insert("future_projection".to_owned(), Value::Bool(true));
    document["files"][0]["projection"]["items"][0]
        .as_object_mut()
        .expect("item object")
        .insert("future_item".to_owned(), Value::from("x"));
    // A `decorator_start_line` is the kind of additive field the docs promise
    // can be added within v1.
    document["files"][0]["outline"][0]["span"]
        .as_object_mut()
        .expect("span object")
        .insert("decorator_start_line".to_owned(), Value::from(1));
    support::schema::validate(&schema, &document)
        .expect("unknown fields must be permitted for forward compatibility");

    // Required fields and value constraints stay strict: removing a required
    // field and breaking an enum must both fail.
    let mut missing = show_json_stdin(&repo, "src/lib.rs", "types", &source);
    missing["files"][0]["outline"][0]
        .as_object_mut()
        .expect("outline item")
        .remove("signature");
    assert!(
        support::schema::validate(&schema, &missing).is_err(),
        "a missing required field must be rejected"
    );

    let mut bad_enum = show_json_stdin(&repo, "src/lib.rs", "types", &source);
    bad_enum["files"][0]["outline"][0]["kind"] = Value::from("not_a_kind");
    assert!(
        support::schema::validate(&schema, &bad_enum).is_err(),
        "an unknown kind must be rejected"
    );
}
