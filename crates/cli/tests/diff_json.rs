mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use support::{TestRepo, codect_in, doc, fixture, stderr, stdout};

fn run(repo: &TestRepo, mode: &str, base: &str, target: &str) -> Value {
    let output = codect_in(
        repo,
        &["diff", "--format", "json", "--mode", mode, base, target],
    )
    .output()
    .expect("run codect");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stdout(&output).contains('\u{1b}'));
    assert!(stdout(&output).ends_with('\n'));
    serde_json::from_str(&stdout(&output)).expect("valid JSON")
}

fn blob(repo: &TestRepo, source: &str) -> String {
    repo.git_ok_stdin(&["hash-object", "-w", "--stdin"], source.as_bytes())
        .trim()
        .to_owned()
}

fn tree(repo: &TestRepo, entries: &[(&[u8], &str)]) -> String {
    let entries = entries
        .iter()
        .map(|(path, object_id)| ((*path).to_vec(), (*object_id).to_owned()))
        .collect::<Vec<_>>();
    tree_object(repo, &entries)
}

// `git mktree` builds one directory level, so recursive trees are assembled
// from single-component entries. Raw path bytes stay untouched, including
// controls and invalid UTF-8; no filesystem name is involved.
fn tree_object(repo: &TestRepo, entries: &[(Vec<u8>, String)]) -> String {
    let mut files: Vec<(&[u8], &str)> = Vec::new();
    let mut directories: BTreeMap<&[u8], Vec<(Vec<u8>, String)>> = BTreeMap::new();
    for (path, object_id) in entries {
        match path.iter().position(|&byte| byte == b'/') {
            Some(slash) => directories
                .entry(&path[..slash])
                .or_default()
                .push((path[slash + 1..].to_vec(), object_id.clone())),
            None => files.push((path, object_id)),
        }
    }

    let mut items: Vec<(&[u8], Vec<u8>)> = Vec::new();
    for (path, object_id) in files {
        let mut line = Vec::new();
        line.extend_from_slice(b"100644 blob ");
        line.extend_from_slice(object_id.as_bytes());
        line.push(b'\t');
        line.extend_from_slice(path);
        items.push((path, line));
    }
    for (name, children) in directories {
        let subtree = tree_object(repo, &children);
        let mut line = Vec::new();
        line.extend_from_slice(b"040000 tree ");
        line.extend_from_slice(subtree.as_bytes());
        line.push(b'\t');
        line.extend_from_slice(name);
        items.push((name, line));
    }
    items.sort_by(|left, right| left.0.cmp(right.0));

    let mut input = Vec::new();
    for (_, mut line) in items {
        input.append(&mut line);
        input.push(0);
    }
    repo.git_ok_stdin(&["mktree", "-z"], &input)
        .trim()
        .to_owned()
}

fn commit_tree(repo: &TestRepo, tree: &str, parent: Option<&str>, message: &str) -> String {
    let mut args = vec!["commit-tree", tree];
    if let Some(parent) = parent {
        args.push("-p");
        args.push(parent);
    }
    args.push("-m");
    args.push(message);
    repo.git_ok(&args).trim().to_owned()
}

#[test]
fn golden_changed_type_document() {
    let repo = TestRepo::init();
    repo.write("lib.rs", "pub struct User { pub id: u32 }\n");
    repo.commit("base");
    repo.write("lib.rs", "pub struct User { pub id: u64 }\n");
    repo.commit("target");
    let mut document = run(&repo, "types", "HEAD~", "HEAD");
    document["base"]["revision"] = json!("BASE");
    document["target"]["revision"] = json!("TARGET");
    document["base"]["id"] = json!("BASE_ID");
    document["target"]["id"] = json!("TARGET_ID");
    document["files"][0]["base"]["snapshot_id"] = json!("BASE_ID");
    document["files"][0]["target"]["snapshot_id"] = json!("TARGET_ID");
    let expected: Value = serde_json::from_str(&fixture("schema/diff-types.json")).unwrap();
    assert_eq!(document, expected);
}

#[test]
fn golden_moved_document_matches_and_validates_against_schema() {
    let repo = TestRepo::init();
    repo.write("lib.rs", "pub struct Moved {\n    pub value: u32,\n}\n");
    repo.commit("base");
    repo.git_ok(&["mv", "lib.rs", "moved.rs"]);
    repo.commit("move");

    let mut document = run(&repo, "types", "HEAD~", "HEAD");
    document["base"]["revision"] = json!("BASE");
    document["target"]["revision"] = json!("TARGET");
    document["base"]["id"] = json!("BASE_ID");
    document["target"]["id"] = json!("TARGET_ID");
    document["files"][0]["base"]["snapshot_id"] = json!("BASE_ID");
    document["files"][0]["target"]["snapshot_id"] = json!("TARGET_ID");

    let expected: Value = serde_json::from_str(&fixture("schema/diff-moved.json")).unwrap();
    assert_eq!(document, expected);
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("golden move validates as v2");
}

#[test]
fn commit_snapshots_and_changed_panes_validate_against_schema() {
    let repo = TestRepo::init();
    repo.write(
        "src/lib.rs",
        "pub struct User { pub id: u32 }\npub fn greet() {}\n",
    );
    let base = repo.commit("base");
    repo.write(
        "src/lib.rs",
        "pub struct User { pub id: u64 }\npub fn greet() {}\n",
    );
    repo.write("src/new.py", "class New:\n    value: int\n");
    let target = repo.commit("target");

    let value = run(&repo, "types", &base, &target);
    assert_eq!(value["schema"], "codect.diff.v2");
    assert_eq!(
        value["base"],
        json!({"kind":"commit", "revision":base, "id":base})
    );
    assert_eq!(
        value["target"],
        json!({"kind":"commit", "revision":target, "id":target})
    );
    assert_eq!(value["files"].as_array().unwrap().len(), 2);
    assert_eq!(value["files"][0]["status"], "modified");
    assert_eq!(value["files"][1]["status"], "added");
    assert!(value["files"][0].get("old_path").is_none());
    assert!(value["files"][1].get("old_path").is_none());
    assert!(value["files"][1]["base"].is_null());
    assert_eq!(value["files"][0]["base"]["snapshot_id"], base);
    assert_eq!(value["files"][0]["target"]["snapshot_id"], target);
    assert_eq!(value["files"][0]["equal"], false);
    assert!(
        value["files"][0]["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("u64")
    );
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &value).expect("valid diff schema");

    let mut invalid = value.clone();
    invalid["files"][0]["base"]["projection"]
        .as_object_mut()
        .unwrap()
        .remove("items");
    assert!(support::schema::validate(&schema, &invalid).is_err());
}

#[test]
fn body_only_changes_and_empty_projections_are_filtered() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn greet() { println!(\"one\"); }\n");
    let base = repo.commit("base");
    repo.write("src/lib.rs", "pub fn greet() { println!(\"two\"); }\n");
    repo.write("src/empty.rs", "// no declarations\n");
    let target = repo.commit("body only");
    assert_eq!(run(&repo, "signatures", &base, &target)["files"], json!([]));
}

#[test]
fn tests_mode_diff_reports_test_changes_and_ignores_the_rest() {
    let repo = TestRepo::init();
    repo.write(
        "src/lib.rs",
        "#[test]\nfn it_works() {\n    assert!(true);\n}\n\npub fn helper() {}\n",
    );
    let base = repo.commit("base");
    // Neither the test body nor the untouched helper is a focused change.
    repo.write(
        "src/lib.rs",
        "#[test]\nfn it_works() {\n    assert!(false);\n}\n\npub fn helper() {}\n",
    );
    let body_only = repo.commit("body only");
    assert_eq!(run(&repo, "tests", &base, &body_only)["files"], json!([]));

    // Renaming the test is a focused change.
    repo.write(
        "src/lib.rs",
        "#[test]\nfn it_also_works() {\n    assert!(false);\n}\n\npub fn helper() {}\n",
    );
    let renamed = repo.commit("rename");
    let document = run(&repo, "tests", &body_only, &renamed);
    let file = &document["files"][0];
    assert_eq!(file["path"], "src/lib.rs");
    assert_eq!(file["status"], "modified");
    assert_eq!(file["base"]["snapshot_id"], body_only);
    assert!(
        file["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("fn it_works();")
    );
    assert!(
        file["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("fn it_also_works();")
    );
    // The unchanged helper is absent from both focused projections but is still
    // located in the mode-independent outline.
    assert!(
        !file["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("helper")
    );
    assert!(
        file["target"]["outline"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |item| item["stable_key"].as_str().unwrap().contains("helper")
                    && item["retained_in_mode"] == json!(false)
            )
    );

    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("valid diff schema");
}

#[test]
fn tests_mode_reports_a_changed_type_as_invisible() {
    // A type-only change is invisible to Tests, visible to Types.
    let repo = TestRepo::init();
    repo.write(
        "src/lib.rs",
        "pub struct S {\n    pub a: u32,\n}\n\n#[test]\nfn it_works() {}\n",
    );
    let base = repo.commit("base");
    repo.write(
        "src/lib.rs",
        "pub struct S {\n    pub a: u64,\n}\n\n#[test]\nfn it_works() {}\n",
    );
    let target = repo.commit("type change");
    assert_eq!(run(&repo, "tests", &base, &target)["files"], json!([]));
    assert_ne!(run(&repo, "types", &base, &target)["files"], json!([]));
}

#[test]
fn deletion_has_absent_target_and_preserved_base_projection() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", "pub struct Gone;\n");
    let base = repo.commit("base");
    repo.remove("src/old.rs");
    let target = repo.commit("delete");
    let document = run(&repo, "types", &base, &target);
    let file = &document["files"][0];
    assert_eq!(file["status"], "deleted");
    assert!(file.get("old_path").is_none());
    assert_eq!(file["target"], Value::Null);
    assert_eq!(file["base"]["snapshot_id"], base);
    assert!(
        file["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("Gone")
    );
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("valid deleted diff schema");
}

#[test]
fn moved_files_emit_v2_old_path_both_sides_and_equal() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", "pub struct Moved {\n    pub id: u32,\n}\n");
    let base = repo.commit("base");
    repo.remove("src/old.rs");
    repo.write("src/new.rs", "pub struct Moved {\n    pub id: u32,\n}\n");
    let target = repo.commit("pure move");

    let document = run(&repo, "types", &base, &target);
    assert_eq!(document["schema"], "codect.diff.v2");
    assert_eq!(document["files"].as_array().unwrap().len(), 1);
    let file = &document["files"][0];
    assert_eq!(file["status"], "moved");
    assert_eq!(file["path"], "src/new.rs");
    assert_eq!(file["old_path"], "src/old.rs");
    assert_eq!(file["language"], "rust");
    assert_eq!(file["equal"], true);
    assert_eq!(file["base"]["snapshot_id"], base);
    assert_eq!(file["target"]["snapshot_id"], target);
    assert_eq!(
        file["base"]["projection"]["text"],
        file["target"]["projection"]["text"]
    );
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("valid moved diff schema");

    // Text mode uses Git-style rename headers and emits no content headers or
    // hunks for a pure move.
    let text = codect_in(
        &repo,
        &[
            "diff", "--format", "text", "--mode", "types", &base, &target,
        ],
    )
    .output()
    .expect("run codect");
    assert!(text.status.success(), "{}", stderr(&text));
    let text = stdout(&text);
    assert_eq!(
        text,
        concat!(
            "diff --codect a/src/old.rs b/src/new.rs\n",
            "rename from src/old.rs\n",
            "rename to src/new.rs\n",
        ),
        "a pure move must not print content headers or hunks"
    );

    // A source edit during a move keeps the move and adds ordinary hunks
    // between the old and new paths.
    repo.remove("src/new.rs");
    repo.write("src/edited.rs", "pub struct Moved {\n    pub id: u64,\n}\n");
    let edited = repo.commit("edited move");
    let document = run(&repo, "types", &target, &edited);
    let file = &document["files"][0];
    assert_eq!(file["status"], "moved");
    assert_eq!(file["old_path"], "src/new.rs");
    assert_eq!(file["path"], "src/edited.rs");
    assert_eq!(file["equal"], false);
    support::schema::validate(&schema, &document).expect("valid edited move schema");

    let text = codect_in(
        &repo,
        &[
            "diff", "--format", "text", "--mode", "types", &target, &edited,
        ],
    )
    .output()
    .expect("run codect");
    assert!(text.status.success(), "{}", stderr(&text));
    let text = stdout(&text);
    assert!(text.contains("rename from src/new.rs\nrename to src/edited.rs\n"));
    assert!(text.contains("--- a/src/new.rs\n+++ b/src/edited.rs\n"));
    assert!(!text.contains("rename from src/old.rs"));
}

#[test]
fn object_tree_move_paths_are_raw_distinct_and_escape_without_controls() {
    let repo = TestRepo::init();
    // The old path contains the literal characters `\xFF`; the new path
    // contains one invalid UTF-8 byte 0xFF. Raw bytes differ but an
    // insufficiently injective escaped form would make both print `\xFF`.
    let literal_path = b"src/\\xFF.rs";
    let invalid_path = b"src/\xFF.rs";
    let control_old = b"src/esc\x1Bape.rs";
    let control_new = b"src/line\nbreak.rs";
    let literal_blob = blob(&repo, "pub struct Literal { pub value: u32 }\n");
    let control_blob = blob(&repo, "pub struct Control { pub value: u32 }\n");

    let base_tree = tree(
        &repo,
        &[
            (literal_path, literal_blob.as_str()),
            (control_old, control_blob.as_str()),
        ],
    );
    let target_tree = tree(
        &repo,
        &[
            (invalid_path, literal_blob.as_str()),
            (control_new, control_blob.as_str()),
        ],
    );
    let base = commit_tree(&repo, &base_tree, None, "base");
    let target = commit_tree(&repo, &target_tree, Some(&base), "move");

    let document = run(&repo, "types", &base, &target);
    assert_eq!(document["schema"], "codect.diff.v2");
    let files = document["files"].as_array().expect("files array");
    assert_eq!(files.len(), 2, "{document}");

    let invalid_move = files
        .iter()
        .find(|file| file["path"] == json!("src/\\xFF.rs"))
        .unwrap_or_else(|| panic!("invalid-byte move missing: {document}"));
    assert_eq!(invalid_move["old_path"], json!("src/\\\\xFF.rs"));
    assert_ne!(
        invalid_move["old_path"], invalid_move["path"],
        "serialized old and new paths must remain distinct"
    );
    let control_move = files
        .iter()
        .find(|file| file["path"] == json!("src/line\\nbreak.rs"))
        .unwrap_or_else(|| panic!("control-path move missing: {document}"));
    assert_eq!(control_move["old_path"], json!("src/esc\\x1Bape.rs"));

    for file in files {
        for key in ["path", "old_path"] {
            let value = file[key].as_str().expect("string path");
            assert!(
                !value.chars().any(char::is_control),
                "{key} must not contain raw controls: {value:?}"
            );
        }
    }
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("raw-path move document is valid v2");

    let output = codect_in(
        &repo,
        &["--color=never", "diff", "--mode", "types", &base, &target],
    )
    .output()
    .expect("run codect");
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(!text.contains('\u{1b}'), "redirected text must be unstyled");
    assert!(!text.contains('\r'), "raw carriage return leaked: {text:?}");
    assert_eq!(
        text.lines().count(),
        6,
        "two pure moves must produce exactly six header lines: {text:?}"
    );
    let headers: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("diff --codect ") || line.starts_with("rename "))
        .collect();
    assert_eq!(headers.len(), 6);
    assert!(
        headers
            .iter()
            .all(|line| !line.chars().any(char::is_control)),
        "header lines must escape controls rather than split: {headers:?}"
    );
    assert!(
        text.lines()
            .any(|line| line == "diff --codect a/src/\\\\xFF.rs b/src/\\xFF.rs"),
        "literal and invalid paths must remain distinct in the header: {text:?}"
    );
    assert!(
        text.lines()
            .any(|line| line == "rename from src/esc\\x1Bape.rs")
    );
    assert!(
        text.lines()
            .any(|line| line == "rename to src/line\\nbreak.rs")
    );
}

#[test]
fn v2_schema_rejects_invalid_move_and_nonmove_shapes() {
    let repo = TestRepo::init();
    repo.write("src/a.rs", "pub struct A;\n");
    let base = repo.commit("base");
    repo.write("src/a.rs", "pub struct A { pub id: u32 }\n");
    repo.write("src/b.py", "class B:\n    value: int\n");
    let target = repo.commit("target");
    let document = run(&repo, "types", &base, &target);
    assert_eq!(document["files"][0]["status"], "modified");
    assert_eq!(document["files"][1]["status"], "added");

    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("baseline is valid");

    // Non-move statuses may not carry `old_path`; only the conditional `not`
    // constraint rejects this document.
    let mut old_path_on_modified = document.clone();
    old_path_on_modified["files"][0]["old_path"] = json!("src/source.rs");
    assert!(
        support::schema::validate(&schema, &old_path_on_modified).is_err(),
        "a modified file must not carry old_path"
    );

    // A move requires old_path and both sides. Each mutation below satisfies
    // every unconditional constraint; only the `then` branch rejects it.
    let mut missing_old_path = document.clone();
    missing_old_path["files"][0]["status"] = json!("moved");
    assert!(
        support::schema::validate(&schema, &missing_old_path).is_err(),
        "a moved file must carry old_path"
    );

    let mut null_base = document.clone();
    null_base["files"][0]["status"] = json!("moved");
    null_base["files"][0]["old_path"] = json!("src/source.rs");
    null_base["files"][0]["base"] = Value::Null;
    assert!(
        support::schema::validate(&schema, &null_base).is_err(),
        "a moved file must carry a base side"
    );

    let mut null_target = document.clone();
    null_target["files"][0]["status"] = json!("moved");
    null_target["files"][0]["old_path"] = json!("src/source.rs");
    null_target["files"][0]["target"] = Value::Null;
    assert!(
        support::schema::validate(&schema, &null_target).is_err(),
        "a moved file must carry a target side"
    );

    // An addition has no base side; this is rejected only by the status-specific
    // conditional branch.
    let mut added_with_base = document.clone();
    let added_target = added_with_base["files"][1]["target"].clone();
    added_with_base["files"][1]["base"] = added_target;
    assert!(
        support::schema::validate(&schema, &added_with_base).is_err(),
        "an added file must not carry a base side"
    );

    // A non-move keeps `equal: false`; the `else` branch owns that constraint.
    let mut equal_on_added = document.clone();
    equal_on_added["files"][1]["equal"] = json!(true);
    assert!(
        support::schema::validate(&schema, &equal_on_added).is_err(),
        "a non-move must keep equal false"
    );

    // The same shape with all move constraints satisfied is valid, proving the
    // negative cases above are not rejected by unrelated constraints. The two
    // projection texts differ, so the semantically correct flag is false.
    let mut valid_move = document.clone();
    valid_move["files"][0]["status"] = json!("moved");
    valid_move["files"][0]["old_path"] = json!("src/source.rs");
    valid_move["files"][0]["equal"] = json!(false);
    valid_move["files"].as_array_mut().unwrap().truncate(1);
    support::schema::validate(&schema, &valid_move).expect("well-formed move schema");
}

#[test]
fn four_languages_have_projected_sides_and_text_mode_is_unchanged() {
    let repo = TestRepo::init();
    for (path, fixture_path) in [
        ("src/App.elm", "elm/type-aliases/input.elm"),
        ("src/Main.hs", "haskell/canonical-types/input.hs"),
        ("src/app.py", "python/canonical-types/input.py"),
        ("src/lib.rs", "rust/structs/input.rs"),
    ] {
        repo.write(path, &fixture(fixture_path));
    }
    let base = repo.commit("all languages");
    for (path, fixture_path) in [
        ("src/App.elm", "elm/type-aliases/input.elm"),
        ("src/Main.hs", "haskell/canonical-types/input.hs"),
        ("src/app.py", "python/canonical-types/input.py"),
        ("src/lib.rs", "rust/structs/input.rs"),
    ] {
        repo.write(
            path,
            &format!(
                "{}\n{}",
                fixture(fixture_path),
                if path.ends_with(".py") {
                    "class Added: pass\n"
                } else if path.ends_with(".rs") {
                    "pub struct Added;\n"
                } else if path.ends_with(".elm") {
                    "type alias Added = Int\n"
                } else {
                    "data Added = Added\n"
                }
            ),
        );
    }
    let target = repo.commit("new types");
    let json = run(&repo, "types", &base, &target);
    let paths: Vec<_> = json["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["src/App.elm", "src/Main.hs", "src/app.py", "src/lib.rs"]
    );
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &json).unwrap();

    let default = codect_in(&repo, &["diff", "--mode", "types", &base, &target])
        .output()
        .unwrap();
    let explicit = codect_in(
        &repo,
        &[
            "diff", "--format", "text", "--mode", "types", &base, &target,
        ],
    )
    .output()
    .unwrap();
    assert_eq!(default.stdout, explicit.stdout);
    assert_eq!(default.stderr, explicit.stderr);
}

#[test]
fn staged_and_unstaged_snapshots_are_distinct() {
    let repo = TestRepo::init();
    repo.write("lib.rs", "pub struct Item { pub value: u8 }\n");
    repo.commit("base");
    repo.write("lib.rs", "pub struct Item { pub value: u16 }\n");
    repo.git_ok(&["add", "lib.rs"]);
    repo.write("lib.rs", "pub struct Item { pub value: u32 }\n");

    let staged = run(&repo, "types", "HEAD", ":index");
    let unstaged = run(&repo, "types", ":index", ":worktree");
    assert_eq!(staged["target"]["kind"], "index");
    assert_eq!(unstaged["base"]["kind"], "index");
    assert_eq!(unstaged["target"]["kind"], "worktree");
    assert!(
        staged["files"][0]["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("u16")
    );
    assert!(
        unstaged["files"][0]["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("u32")
    );
    assert!(
        !unstaged["files"][0]["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("u8")
    );
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &staged).unwrap();
    support::schema::validate(&schema, &unstaged).unwrap();
}

#[test]
fn root_commit_compares_to_empty_tree_and_missing_worktree_side() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub struct First;\n");
    repo.write("src/empty.py", "# no declarations\n");
    repo.commit("root");
    let empty_sha = repo.git_ok(&["hash-object", "-t", "tree", "--stdin"]);
    let root = run(&repo, "types", empty_sha.trim(), "HEAD");
    assert_eq!(root["base"]["kind"], "empty");
    assert_eq!(root["files"].as_array().unwrap().len(), 1);
    assert_eq!(root["files"][0]["status"], "added");
    repo.remove("src/lib.rs");
    let deleted = run(&repo, "types", ":index", ":worktree");
    assert_eq!(deleted["files"][0]["status"], "deleted");
    assert!(deleted["files"][0]["target"].is_null());
}

#[test]
fn sha256_root_commit_uses_empty_tree_snapshot() {
    let Some(repo) = TestRepo::init_sha256() else {
        return;
    };
    repo.write("src/lib.rs", "pub struct First;\n");
    repo.commit("root");
    let empty_sha = repo.git_ok(&["hash-object", "-t", "tree", "--stdin"]);
    let root = run(&repo, "types", empty_sha.trim(), "HEAD");
    assert_eq!(root["base"]["kind"], "empty");
    assert_eq!(root["files"][0]["status"], "added");
}

#[test]
fn untracked_files_join_the_worktree_side_and_ignored_files_stay_out() {
    let repo = TestRepo::init();
    repo.write(".gitignore", "target/\n");
    repo.write("src/lib.rs", "pub struct Base;\n");
    repo.commit("base");

    repo.write("src/new.rs", "pub struct New;\n");
    repo.write("target/junk.rs", "pub struct Junk;\n");
    repo.write("notes.txt", "unsupported\n");

    let document = run(&repo, "types", ":index", ":worktree");
    let paths: Vec<_> = document["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["src/new.rs"],
        "an untracked supported file is added; ignored and unsupported files are omitted"
    );
    let file = &document["files"][0];
    assert_eq!(file["status"], "added");
    assert!(file["base"].is_null());
    assert_eq!(file["target"]["snapshot_id"], ":worktree");
    assert!(
        file["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("New")
    );

    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).unwrap();
}

#[test]
fn text_diff_accepts_snapshots_and_renders_untracked_files() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn base() {}\n");
    repo.commit("base");
    repo.write("src/lib.rs", "pub fn staged() {}\n");
    repo.git_ok(&["add", "src/lib.rs"]);
    repo.write("src/lib.rs", "pub fn unstaged() {}\n");
    repo.write("src/fresh.rs", "pub struct Fresh;\n");

    let staged = codect_in(
        &repo,
        &[
            "diff",
            "--format",
            "text",
            "--mode",
            "signatures",
            "HEAD",
            ":index",
        ],
    )
    .output()
    .expect("run codect");
    assert!(staged.status.success(), "{}", stderr(&staged));
    let staged = stdout(&staged);
    assert!(staged.contains("src/lib.rs"));
    assert!(staged.contains("+pub fn staged();"));

    let unstaged = codect_in(
        &repo,
        &[
            "diff",
            "--format",
            "text",
            "--mode",
            "signatures",
            ":index",
            ":worktree",
        ],
    )
    .output()
    .expect("run codect");
    assert!(unstaged.status.success(), "{}", stderr(&unstaged));
    let unstaged = stdout(&unstaged);
    assert!(unstaged.contains("+pub fn unstaged();"));
    assert!(unstaged.contains("diff --codect a/src/fresh.rs b/src/fresh.rs"));
    assert!(unstaged.contains("+pub struct Fresh;"));
}

#[test]
fn merge_base_diff_omits_base_branch_commits_and_marks_the_snapshot() {
    let repo = TestRepo::init();
    repo.write("a.rs", "pub struct Base;\n");
    let fork = repo.commit("fork point");
    repo.git_ok(&["branch", "feature"]);

    // Advance `main` after the fork, then advance `feature` independently.
    repo.write("a.rs", "pub struct Base;\npub struct MainOnly;\n");
    repo.commit("main only");
    repo.git_ok(&["checkout", "-q", "feature"]);
    repo.write("a.rs", "pub struct Base;\npub struct FeatureOnly;\n");
    repo.commit("feature only");

    let output = codect_in(
        &repo,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "types",
            "--merge-base",
            "main",
            "feature",
        ],
    )
    .output()
    .expect("run codect");
    assert!(output.status.success(), "{}", stderr(&output));
    let document: Value = serde_json::from_str(&stdout(&output)).expect("valid JSON");

    // The base echoes the requested revision, resolves to the fork point, and
    // is marked so a consumer can tell it is not the tip of `main`.
    assert_eq!(document["base"]["revision"], "main");
    assert_eq!(document["base"]["merge_base"], true);
    assert_eq!(document["base"]["id"], fork);
    assert_eq!(document["base"]["kind"], "commit");
    assert!(document["target"].get("merge_base").is_none());

    let file = &document["files"][0];
    assert_eq!(file["path"], "a.rs");
    assert!(
        !file["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("MainOnly"),
        "the merge-base base omits main's declaration"
    );
    assert!(
        file["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("FeatureOnly")
    );

    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json")).unwrap();
    support::schema::validate(&schema, &document).expect("valid diff schema");

    // Without the flag the tip-to-tip diff still shows main's declaration, and
    // no marker is emitted.
    let tip = run(&repo, "types", "main", "feature");
    assert!(tip["base"].get("merge_base").is_none());
    assert!(
        tip["files"][0]["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("MainOnly")
    );

    // Text mode produces the same file set with no base-branch noise.
    let text = codect_in(
        &repo,
        &[
            "diff",
            "--format",
            "text",
            "--mode",
            "types",
            "--merge-base",
            "main",
            "feature",
        ],
    )
    .output()
    .expect("run codect");
    assert!(text.status.success(), "{}", stderr(&text));
    let text = stdout(&text);
    assert!(text.contains("FeatureOnly"));
    assert!(!text.contains("MainOnly"));
}

#[test]
fn merge_base_rejects_a_snapshot_side_as_a_usage_error() {
    let repo = TestRepo::init();
    repo.write("a.rs", "pub struct A;\n");
    repo.commit("base");

    let output = codect_in(
        &repo,
        &["diff", "--mode", "types", "--merge-base", "HEAD", ":index"],
    )
    .output()
    .expect("run codect");
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("--merge-base"));
    assert!(stdout(&output).is_empty());
}
