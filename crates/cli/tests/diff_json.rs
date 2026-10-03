mod support;

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
    assert_eq!(value["schema"], "codect.diff.v1");
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
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v1.json")).unwrap();
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
fn deletion_has_absent_target_and_preserved_base_projection() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", "pub struct Gone;\n");
    let base = repo.commit("base");
    repo.remove("src/old.rs");
    let target = repo.commit("delete");
    let document = run(&repo, "types", &base, &target);
    let file = &document["files"][0];
    assert_eq!(file["status"], "deleted");
    assert_eq!(file["target"], Value::Null);
    assert_eq!(file["base"]["snapshot_id"], base);
    assert!(
        file["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("Gone")
    );
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
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v1.json")).unwrap();
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
    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v1.json")).unwrap();
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
