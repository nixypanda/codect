// Black-box acceptance tests for file move detection.
//
// These tests intentionally describe the published `codect.diff.v2` contract
// and move-detection behavior, not the matcher implementation. They create
// real temporary repositories with the `git` executable through the existing
// test support module; the binary under test never invokes Git.
//
// These are independent acceptance tests for the shipped `codect.diff.v2`
// document and the existing fatal error contract.

mod support;

use std::fs;
use std::io;

use serde_json::Value;
use support::{TestRepo, codect_in, doc, stderr, stdout};

const RUST_SMALL: &str = "\
pub struct Small {
    pub value: u32,
}
";

const RUST_OTHER: &str = "\
pub struct Other {
    pub value: u32,
}
";

const RUST_OLD: &str = "\
pub struct Moved {
    pub value: u32,
}

pub fn helper() -> u32 {
    1
}
";

const RUST_TYPE_EDIT: &str = "\
pub struct Moved {
    pub value: u64,
}

pub fn helper() -> u32 {
    1
}
";

const RUST_BODY_EDIT: &str = "\
pub struct Moved {
    pub value: u32,
}

pub fn helper() -> u32 {
    2
}
";

const PYTHON_OLD: &str = "\
class Widget:
    value: int
";

fn run(repo: &TestRepo, args: &[&str]) -> std::process::Output {
    codect_in(repo, args).output().expect("run codect")
}

fn run_json(repo: &TestRepo, args: &[&str]) -> Value {
    let output = run(repo, args);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains('\u{1b}'),
        "JSON output must not contain ANSI"
    );
    assert!(
        stdout(&output).ends_with('\n'),
        "JSON must end with one newline"
    );
    serde_json::from_str(&stdout(&output)).expect("stdout is a JSON document")
}

fn diff_json(repo: &TestRepo, mode: &str, base: &str, target: &str) -> Value {
    run_json(
        repo,
        &["diff", "--format", "json", "--mode", mode, base, target],
    )
}

fn run_text(repo: &TestRepo, mode: &str, base: &str, target: &str) -> String {
    let output = run(repo, &["diff", "--mode", mode, base, target]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        !stdout(&output).contains('\u{1b}'),
        "redirected text must not contain ANSI"
    );
    stdout(&output)
}

fn assert_schema_v2(document: &Value) {
    assert_eq!(
        document["schema"], "codect.diff.v2",
        "diff moved to v2 for move events: {document}"
    );
}

fn files(document: &Value) -> &[Value] {
    document["files"]
        .as_array()
        .expect("files must be an array")
}

fn file_at<'a>(document: &'a Value, path: &str) -> &'a Value {
    files(document)
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("no file event for {path:?}: {document}"))
}

fn statuses(document: &Value) -> Vec<&str> {
    files(document)
        .iter()
        .map(|file| file["status"].as_str().expect("status string"))
        .collect()
}

fn assert_moved(file: &Value, old: &str, new: &str, equal: bool) {
    assert_eq!(file["status"], "moved", "expected a move: {file}");
    assert_eq!(file["path"], new, "move path is the target path");
    assert_eq!(file["old_path"], old, "move source path");
    assert_eq!(file["equal"], equal, "move equal flag");
    assert_ne!(
        file["old_path"], file["path"],
        "a move must name distinct paths"
    );
    assert!(file["base"].is_object(), "moved base side: {file}");
    assert!(file["target"].is_object(), "moved target side: {file}");
}

fn assert_no_old_path(file: &Value) {
    assert!(
        file.get("old_path").is_none(),
        "old_path is only present for moves: {file}"
    );
}

fn exact_move_repo() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("old.rs", RUST_OLD);
    repo.commit("base");
    repo.git_ok(&["mv", "old.rs", "new.rs"]);
    repo.commit("move");
    repo
}

fn worktree_move(repo: &TestRepo, old: &str, new: &str) {
    let destination = repo.path().join(new);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).expect("create destination directory");
    }
    fs::rename(repo.path().join(old), destination).expect("move worktree file");
}

fn write_bytes(repo: &TestRepo, relative: &str, bytes: &[u8]) {
    let path = repo.path().join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, bytes).expect("write fixture bytes");
}

fn large_rust_source(min_bytes: usize) -> String {
    let mut source = String::from("pub struct Large { pub value: u32 }\n");
    let mut index = 0usize;
    while source.len() < min_bytes {
        source.push_str(&format!("pub fn function_{index}() {{}}\n"));
        index += 1;
    }
    source
}

// Produces an exact-size valid source made of a declaration plus comment
// lines. Old and new variants use the same size and differ only in the marker
// on the first comment line, so line-unit similarity stays high above 4 KiB.
fn padded_comment_source(is_python: bool, size: usize, marker: &str) -> String {
    let (header, prefix, fill) = if is_python {
        ("class Sized:\n    pass\n", "# ", 'p')
    } else {
        ("pub struct Sized;\n", "// ", 'r')
    };
    let min_tail = prefix.len() + 2;
    assert!(
        size > header.len() + min_tail + prefix.len() + marker.len() + 1,
        "size fixture must leave room for a declaration and comments"
    );
    let first_len = 40.min(size - header.len() - min_tail);
    assert!(first_len > prefix.len() + marker.len());

    let mut source = String::with_capacity(size);
    source.push_str(header);
    source.push_str(prefix);
    source.push_str(marker);
    while source.len() < header.len() + first_len - 1 {
        source.push(fill);
    }
    source.push('\n');

    let remaining = size - source.len();
    let full_lines = (remaining - min_tail) / 40;
    for _ in 0..full_lines {
        let line_start = source.len();
        source.push_str(prefix);
        while source.len() < line_start + 39 {
            source.push(fill);
        }
        source.push('\n');
    }

    let tail = size - source.len();
    assert!(tail >= min_tail);
    source.push_str(prefix);
    while source.len() < size - 1 {
        source.push(fill);
    }
    source.push('\n');
    assert_eq!(source.len(), size, "size fixture must be exact");
    source
}

// Prefix ends in a newline; both comments share `// ` and the final newline.
// The common units are therefore prefix + `// ` + newline.
fn score_source(prefix: &str, suffix_len: usize, marker: char) -> String {
    let mut source = String::from(prefix);
    source.push_str("// ");
    source.extend(std::iter::repeat_n(marker, suffix_len));
    source.push('\n');
    source
}

fn boundary_source(total: usize, crlf: bool) -> String {
    let newline = if crlf { "\r\n" } else { "\n" };
    let header = format!("pub struct Boundary {{ pub value: u32 }}{newline}// ");
    let suffix = total
        .checked_sub(header.len() + newline.len())
        .expect("boundary source is large enough");
    let mut source = header;
    source.extend(std::iter::repeat_n('a', suffix));
    source.push_str(newline);
    assert_eq!(source.len(), total, "boundary source must be exact");
    source
}

fn unicode_comment(marker: &str) -> String {
    format!(
        "pub struct Unicode {{ pub value: u32 }}\n// {}\n",
        marker.repeat(1000)
    )
}

// 16 size/language partitions of 16 endpoints each. Within a partition every
// old/new pair is eligible, but cross-partition pairs fail the language or
// size-ratio filter. This gives 256 endpoints per side and exactly 4,096
// eligible pairs. With `extra_cross`, one cross-language endpoint pair raises
// each side to 257 raw endpoints without adding an eligible pair.
fn partitioned_side_cap_repo(extra_cross: bool) -> TestRepo {
    const PER_PARTITION: usize = 16;
    const RUST_SIZES: [usize; 12] = [
        64, 130, 265, 535, 1075, 2155, 4315, 8635, 17275, 34555, 69115, 138235,
    ];
    const PYTHON_SIZES: [usize; 4] = [64, 130, 265, 535];

    let repo = TestRepo::init();
    let mut old_paths = Vec::new();
    let mut new_paths = Vec::new();

    let mut add_partition =
        |repo: &TestRepo, prefix: &str, extension: &str, python: bool, size: usize| {
            for index in 0..PER_PARTITION {
                let old = format!("src/{prefix}_{size}_{index:02}_old.{extension}");
                let new = format!("src/{prefix}_{size}_{index:02}_new.{extension}");
                repo.write(
                    &old,
                    &padded_comment_source(python, size, &format!("o{index:02}")),
                );
                old_paths.push(old);
                new_paths.push(new);
            }
        };

    for size in RUST_SIZES {
        add_partition(&repo, "rust", "rs", false, size);
    }
    for size in PYTHON_SIZES {
        add_partition(&repo, "python", "py", true, size);
    }
    if extra_cross {
        // Both endpoints are over the fuzzy byte cap and have no same-language
        // partner, so this adds two raw side-cap endpoints but no eligible
        // pairs. This isolates the raw side cap from the pair cap.
        repo.write(
            "src/cross_old.rs",
            &padded_comment_source(false, 300_000, "cross-old"),
        );
        old_paths.push("src/cross_old.rs".to_owned());
        new_paths.push("src/cross_new.py".to_owned());
    }
    repo.commit("base");

    for (old, new) in old_paths.iter().zip(&new_paths) {
        fs::remove_file(repo.path().join(old)).expect("remove old partition source");
        let python = new.ends_with(".py");
        let size = old
            .strip_prefix("src/")
            .and_then(|rest| rest.split('_').nth(1))
            .and_then(|raw| raw.parse::<usize>().ok());
        let index = old
            .strip_prefix("src/")
            .and_then(|rest| rest.rsplit('_').nth(1))
            .and_then(|raw| raw.parse::<usize>().ok());
        let contents = match (size, index) {
            (Some(size), Some(index)) => {
                padded_comment_source(python, size, &format!("n{index:02}"))
            }
            _ => {
                assert!(extra_cross && old == "src/cross_old.rs");
                padded_comment_source(true, 300_000, "cross-new")
            }
        };
        repo.write(new, &contents);
    }
    repo.commit("moved");
    repo
}

fn python_source(class_name: &str) -> String {
    format!(
        "class {class_name}:\n    value: int\n\n    def method(self) -> int:\n        return 1\n# {}\n",
        "python-shared-".repeat(80)
    )
}

fn rust_candidate_source(struct_name: &str, field_type: &str) -> String {
    format!(
        "pub struct {struct_name} {{ pub value: {field_type} }}\n// {}\n",
        "rust-shared-".repeat(80)
    )
}

// Every old source is similar to every new source in its language, every pair
// is under the size cap and above the ratio floor, and old/new sources differ
// so no pair is exact. The raw cross-product is (python + rust)^2, while the
// language-filtered pair count is python^2 + rust^2.
fn py_rust_pair_repo(python_count: usize, rust_count: usize) -> TestRepo {
    let repo = TestRepo::init();
    for index in 0..python_count {
        repo.write(
            &format!("src/py_old_{index:03}.py"),
            &python_source(&format!("PyOld{index:03}")),
        );
    }
    for index in 0..rust_count {
        repo.write(
            &format!("src/rust_old_{index}.rs"),
            &rust_candidate_source(&format!("ROld{index}"), "u32"),
        );
    }
    repo.commit("base");
    for index in 0..python_count {
        fs::remove_file(repo.path().join(format!("src/py_old_{index:03}.py")))
            .expect("remove old Python candidate");
        repo.write(
            &format!("src/py_new_{index:03}.py"),
            &python_source(&format!("PyNew{index:03}")),
        );
    }
    for index in 0..rust_count {
        fs::remove_file(repo.path().join(format!("src/rust_old_{index}.rs")))
            .expect("remove old Rust candidate");
        repo.write(
            &format!("src/rust_new_{index}.rs"),
            &rust_candidate_source(&format!("RNew{index}"), "u64"),
        );
    }
    repo.commit("edited moves");
    repo
}

#[test]
fn exact_commit_move_is_one_v2_event_and_pure_text() {
    let repo = exact_move_repo();
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 1, "{document}");
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);

    let text = run_text(&repo, "types", "HEAD~1", "HEAD");
    assert_eq!(
        text,
        "diff --codect a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n"
    );
    assert!(
        !text.contains("--- a/old.rs"),
        "pure move has no content header"
    );
    assert!(!text.contains("/dev/null"), "pure move has no absent side");
}

#[test]
fn exact_index_and_worktree_moves_are_detected() {
    let committed = exact_move_repo();
    for (base, target) in [("HEAD~1", ":index"), ("HEAD~1", ":worktree")] {
        let document = diff_json(&committed, "types", base, target);
        assert_schema_v2(&document);
        assert_eq!(files(&document).len(), 1, "{base}..{target}: {document}");
        assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
    }

    let unstaged = TestRepo::init();
    unstaged.write("old.rs", RUST_OLD);
    unstaged.commit("base");
    worktree_move(&unstaged, "old.rs", "new.rs");
    for (base, target) in [("HEAD", ":worktree"), (":index", ":worktree")] {
        let document = diff_json(&unstaged, "types", base, target);
        assert_schema_v2(&document);
        assert_eq!(files(&document).len(), 1, "{base}..{target}: {document}");
        assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
    }
}

#[test]
fn same_path_pairs_take_precedence_over_a_move() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_SMALL);
    repo.commit("base");
    repo.write("a.rs", RUST_TYPE_EDIT);
    repo.write("b.rs", RUST_SMALL);
    repo.commit("modified and copied");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(statuses(&document), ["modified", "added"], "{document}");
    assert_no_old_path(file_at(&document, "a.rs"));
    assert_no_old_path(file_at(&document, "b.rs"));
}

#[test]
fn identical_new_files_do_not_produce_copies() {
    let repo = TestRepo::init();
    repo.write("source.rs", RUST_SMALL);
    repo.commit("base");
    repo.remove("source.rs");
    repo.write("first.rs", RUST_SMALL);
    repo.write("second.rs", RUST_SMALL);
    repo.commit("copy twice");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    let moved: Vec<&Value> = files(&document)
        .iter()
        .filter(|file| file["status"] == "moved")
        .collect();
    assert_eq!(moved.len(), 1, "a copy is not a second move: {document}");
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "added")
            .count(),
        1,
        "{document}"
    );
}

#[test]
fn ambiguous_identical_sources_pair_by_raw_path_bytes() {
    let repo = TestRepo::init();
    repo.write("old_a.rs", RUST_SMALL);
    repo.write("old_b.rs", RUST_SMALL);
    repo.commit("base");
    repo.remove("old_a.rs");
    repo.remove("old_b.rs");
    repo.write("new_z.rs", RUST_SMALL);
    repo.write("new_y.rs", RUST_SMALL);
    repo.commit("ambiguous moves");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 2, "{document}");
    assert_moved(file_at(&document, "new_y.rs"), "old_a.rs", "new_y.rs", true);
    assert_moved(file_at(&document, "new_z.rs"), "old_b.rs", "new_z.rs", true);
}

#[test]
fn edited_rename_is_a_move_with_focused_hunks() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", RUST_OLD);
    repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.write("src/new.rs", RUST_TYPE_EDIT);
    repo.commit("edited move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 1, "{document}");
    assert_moved(
        file_at(&document, "src/new.rs"),
        "src/old.rs",
        "src/new.rs",
        false,
    );
    let file = file_at(&document, "src/new.rs");
    assert!(
        file["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("u32"),
        "{file}"
    );
    assert!(
        file["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("u64"),
        "{file}"
    );

    let text = run_text(&repo, "types", "HEAD~1", "HEAD");
    assert!(
        text.contains("diff --codect a/src/old.rs b/src/new.rs\n"),
        "{text}"
    );
    assert!(text.contains("rename from src/old.rs\n"), "{text}");
    assert!(text.contains("rename to src/new.rs\n"), "{text}");
    assert!(text.contains("--- a/src/old.rs\n"), "{text}");
    assert!(text.contains("+++ b/src/new.rs\n"), "{text}");
    assert!(text.contains("@@"), "{text}");
    assert!(!text.contains("/dev/null"), "{text}");
}

#[test]
fn body_only_rename_has_no_focused_hunks() {
    let repo = TestRepo::init();
    repo.write("old.rs", RUST_OLD);
    repo.commit("base");
    repo.git_ok(&["mv", "old.rs", "new.rs"]);
    repo.write("new.rs", RUST_BODY_EDIT);
    repo.commit("body-only move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
    let text = run_text(&repo, "types", "HEAD~1", "HEAD");
    assert!(text.contains("rename from old.rs\n"));
    assert!(
        !text.contains("@@"),
        "body-only move has no focused hunk: {text}"
    );
}

#[test]
fn unrelated_files_with_the_same_projection_do_not_pair() {
    let repo = TestRepo::init();
    repo.write(
        "old.rs",
        &format!(
            "pub struct Same {{ pub value: u32 }}\n// {}\n",
            "x".repeat(1000)
        ),
    );
    repo.commit("base");
    repo.remove("old.rs");
    repo.write(
        "new.rs",
        &format!(
            "pub struct Same {{ pub value: u32 }}\n// {}\n",
            "y".repeat(1000)
        ),
    );
    repo.commit("unrelated same projection");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "source bytes, not projection text, decide identity: {document}"
    );
    assert_eq!(statuses(&document).len(), 2, "{document}");
}

#[test]
fn moves_do_not_cross_language_adapters_but_python_suffixes_are_eligible() {
    let cross = TestRepo::init();
    let long_name = "A".repeat(500);
    cross.write("old.rs", &format!("pub struct {long_name};\n"));
    cross.commit("base");
    cross.remove("old.rs");
    cross.write("new.py", &format!("class {long_name}:\n    pass\n"));
    cross.commit("cross-language");
    let document = diff_json(&cross, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "different language adapters must not pair: {document}"
    );

    let python = TestRepo::init();
    python.write("old.py", PYTHON_OLD);
    python.commit("base");
    python.git_ok(&["mv", "old.py", "new.pyi"]);
    python.commit("python stub move");
    let document = diff_json(&python, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.pyi"), "old.py", "new.pyi", true);
}

#[test]
fn unsupported_file_moves_are_invisible() {
    let repo = TestRepo::init();
    repo.write("old.txt", "same text\n");
    repo.commit("base");
    repo.git_ok(&["mv", "old.txt", "new.txt"]);
    repo.commit("unsupported move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        files(&document).is_empty(),
        "unsupported files produce no diff events: {document}"
    );
}

#[cfg(unix)]
#[test]
fn submodule_moves_are_not_paired() {
    let source = TestRepo::init();
    source.write("lib.rs", RUST_SMALL);
    source.commit("submodule content");

    let repo = TestRepo::init();
    let source_path = source.path().to_str().expect("UTF-8 temp path");
    let added = repo.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        source_path,
        "old.rs",
    ]);
    if !added.status.success() {
        eprintln!("skipping submodule move test: local submodule setup is unavailable");
        return;
    }
    repo.commit("base");
    let moved = repo.git(&["mv", "old.rs", "new.rs"]);
    if !moved.status.success() {
        eprintln!("skipping submodule move test: git cannot move the gitlink");
        return;
    }
    repo.commit("submodule move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "submodules must never pair: {document}"
    );
}

#[cfg(unix)]
#[test]
fn symlink_moves_are_not_paired() {
    use std::os::unix::fs::symlink;

    let repo = TestRepo::init();
    repo.write("target.txt", "not source\n");
    symlink("target.txt", repo.path().join("old.rs")).expect("create symlink");
    repo.commit("base");
    fs::remove_file(repo.path().join("old.rs")).expect("remove symlink");
    symlink("target.txt", repo.path().join("new.rs")).expect("create symlink");
    repo.commit("symlink move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "symlinks must never pair: {document}"
    );
}

#[test]
fn exact_move_over_256_kib_is_detected_in_commits_and_worktree() {
    let source = large_rust_source(270_000);
    assert!(source.len() > 256 * 1024);

    let commits = TestRepo::init();
    commits.write("old.rs", &source);
    commits.commit("base");
    commits.git_ok(&["mv", "old.rs", "new.rs"]);
    commits.commit("large move");
    let document = diff_json(&commits, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);

    let worktree = TestRepo::init();
    worktree.write("old.rs", &source);
    worktree.commit("base");
    worktree_move(&worktree, "old.rs", "new.rs");
    let document = diff_json(&worktree, "types", "HEAD", ":worktree");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
}

#[test]
fn merge_base_move_uses_the_fork_point() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", RUST_OLD);
    let fork = repo.commit("fork");
    repo.git_ok(&["branch", "feature"]);
    repo.write("src/old.rs", RUST_TYPE_EDIT);
    let main = repo.commit("main only");
    repo.git_ok(&["checkout", "-q", "feature"]);
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.commit("feature move");

    let document = run_json(
        &repo,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "types",
            "--merge-base",
            main.as_str(),
            "feature",
        ],
    );
    assert_schema_v2(&document);
    assert_eq!(
        document["base"]["id"].as_str(),
        Some(fork.as_str()),
        "{document}"
    );
    assert_moved(
        file_at(&document, "src/new.rs"),
        "src/old.rs",
        "src/new.rs",
        true,
    );
}

#[test]
fn reversal_swaps_old_and_new_paths() {
    let repo = exact_move_repo();
    let document = diff_json(&repo, "types", "HEAD", "HEAD~1");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "old.rs"), "new.rs", "old.rs", true);
}

#[test]
fn empty_side_does_not_create_false_moves() {
    let repo = exact_move_repo();

    let added = diff_json(&repo, "types", ":empty", "HEAD");
    assert_schema_v2(&added);
    assert!(
        !statuses(&added).contains(&"moved"),
        ":empty has no old side: {added}"
    );
    assert!(statuses(&added).iter().all(|status| *status == "added"));

    let deleted = diff_json(&repo, "types", "HEAD", ":empty");
    assert_schema_v2(&deleted);
    assert!(
        !statuses(&deleted).contains(&"moved"),
        ":empty has no target side: {deleted}"
    );
    assert!(statuses(&deleted).iter().all(|status| *status == "deleted"));
}

#[test]
fn old_or_new_scope_returns_the_whole_move_and_area_scope_does_too() {
    let repo = exact_move_repo();

    for path in ["old.rs", "new.rs"] {
        let document = run_json(
            &repo,
            &[
                "diff", "--format", "json", "--mode", "types", "--path", path, "HEAD~1", "HEAD",
            ],
        );
        assert_schema_v2(&document);
        assert_eq!(files(&document).len(), 1, "{path}: {document}");
        assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
    }

    repo.write(
        ".codect.toml",
        "[areas]\nold = [\"old.rs\"]\nnew = [\"new.rs\"]\n",
    );
    for area in ["old", "new"] {
        let document = run_json(
            &repo,
            &[
                "diff", "--format", "json", "--mode", "types", "--area", area, "HEAD~1", "HEAD",
            ],
        );
        assert_schema_v2(&document);
        assert_eq!(files(&document).len(), 1, "{area}: {document}");
        assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
    }
}

#[test]
fn a_scope_that_names_neither_side_is_fatal_without_output() {
    let repo = exact_move_repo();
    let output = run(
        &repo,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "types",
            "--path",
            "missing.rs",
            "HEAD~1",
            "HEAD",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "fatal output must be empty");
    assert!(
        !output.stderr.is_empty(),
        "a fatal error needs a diagnostic"
    );
    assert!(!stderr(&output).contains('\u{1b}'));
}

#[test]
fn moves_sort_by_target_path_in_raw_byte_order() {
    let repo = TestRepo::init();
    repo.write("one.rs", RUST_SMALL);
    repo.write("two.rs", RUST_OTHER);
    repo.write("three.rs", "pub struct Third { pub value: u32 }\n");
    repo.commit("base");
    fs::create_dir_all(repo.path().join("dst")).expect("create dst");
    repo.git_ok(&["mv", "one.rs", "dst/z.rs"]);
    repo.git_ok(&["mv", "two.rs", "dst/A.rs"]);
    repo.git_ok(&["mv", "three.rs", "dst/é.rs"]);
    repo.commit("sorted moves");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    let paths: Vec<&str> = files(&document)
        .iter()
        .map(|file| file["path"].as_str().expect("path"))
        .collect();
    assert_eq!(paths, ["dst/A.rs", "dst/z.rs", "dst/é.rs"], "{document}");

    let text = run_text(&repo, "types", "HEAD~1", "HEAD");
    let a = text.find("b/dst/A.rs").expect("A target block");
    let z = text.find("b/dst/z.rs").expect("z target block");
    let e = text.find("b/dst/é.rs").expect("é target block");
    assert!(
        a < z && z < e,
        "text blocks follow target byte order: {text}"
    );
}

#[test]
fn paths_with_spaces_and_quotes_are_preserved() {
    let repo = TestRepo::init();
    repo.write("src/old file.rs", RUST_SMALL);
    repo.commit("base");
    fs::create_dir_all(repo.path().join("src")).expect("create src");
    repo.git_ok(&["mv", "src/old file.rs", "src/new \"file\".rs"]);
    repo.commit("quoted move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(
        file_at(&document, "src/new \"file\".rs"),
        "src/old file.rs",
        "src/new \"file\".rs",
        true,
    );
    let text = run_text(&repo, "types", "HEAD~1", "HEAD");
    assert!(
        text.contains("diff --codect a/src/old file.rs b/src/new \"file\".rs\n"),
        "{text}"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_use_the_existing_escaped_json_form() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let repo = TestRepo::init();
    let old = b"src/\xFF/old.rs";
    let new = b"src/\xFE/new.rs";
    let old_path = repo.path().join(OsStr::from_bytes(old));
    let new_path = repo.path().join(OsStr::from_bytes(new));
    let prepared = (|| -> io::Result<()> {
        fs::create_dir_all(old_path.parent().expect("old parent"))?;
        fs::write(&old_path, RUST_SMALL.as_bytes())?;
        Ok(())
    })();
    if prepared.is_err() {
        eprintln!("skipping non-UTF-8 path test: the filesystem rejects invalid UTF-8 names");
        return;
    }
    repo.commit("base");

    let prepared = (|| -> io::Result<()> {
        fs::remove_file(&old_path)?;
        fs::create_dir_all(new_path.parent().expect("new parent"))?;
        fs::write(&new_path, RUST_SMALL.as_bytes())?;
        Ok(())
    })();
    if prepared.is_err() {
        eprintln!("skipping non-UTF-8 path test: the filesystem rejects invalid UTF-8 names");
        return;
    }
    repo.commit("raw path move");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    let old_display = "src/\\xFF/old.rs";
    let new_display = "src/\\xFE/new.rs";
    assert_moved(
        file_at(&document, new_display),
        old_display,
        new_display,
        true,
    );
    let text = run_text(&repo, "types", "HEAD~1", "HEAD");
    assert!(
        text.contains(&format!("rename from {old_display}")),
        "{text}"
    );
    assert!(text.contains(&format!("rename to {new_display}")), "{text}");
}

#[test]
fn score_exactly_50_pairs_and_49_does_not() {
    let prefix = "pub struct Score { pub value: u32 }\n";
    let common = prefix.len() + 4;

    let fifty_old = score_source(prefix, common, 'A');
    let fifty_new = score_source(prefix, common, 'B');
    assert_eq!(fifty_old.len(), fifty_new.len());
    let repo = TestRepo::init();
    repo.write("old.rs", &fifty_old);
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &fifty_new);
    repo.commit("score fifty");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);

    let old_extra = common / 20 + 1;
    let new_extra = common * 26 / 25;
    assert_eq!(
        100 * common / (common + new_extra),
        49,
        "fixture must score exactly 49 before flooring"
    );
    let forty_nine_old = score_source(prefix, old_extra, 'A');
    let forty_nine_new = score_source(prefix, new_extra, 'B');
    assert!(
        forty_nine_old.len() * 2 >= forty_nine_new.len(),
        "fixture must pass the size-ratio prefilter"
    );
    let repo = TestRepo::init();
    repo.write("old.rs", &forty_nine_old);
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &forty_nine_new);
    repo.commit("score forty-nine");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "49 must not pair: {document}"
    );
}

#[test]
fn four_kib_boundary_switches_from_scalar_to_line_units() {
    let at_limit_old = boundary_source(4096, true);
    let at_limit_new = boundary_source(4096, false);
    let repo = TestRepo::init();
    repo.write("old.rs", &at_limit_old);
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &at_limit_new);
    repo.commit("at 4 KiB");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);

    let over_old = boundary_source(4097, true);
    let over_new = boundary_source(4097, false);
    let repo = TestRepo::init();
    repo.write("old.rs", &over_old);
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &over_new);
    repo.commit("over 4 KiB");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "over 4 KiB uses complete lines, so CRLF-only differences do not pair: {document}"
    );
}

#[test]
fn unicode_scalar_units_do_not_turn_shared_utf8_bytes_into_a_move() {
    let old = unicode_comment("é");
    let new = unicode_comment("ê");
    assert!(old.len() <= 4096 && new.len() <= 4096);

    let repo = TestRepo::init();
    repo.write("old.rs", &old);
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &new);
    repo.commit("unicode difference");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "scalar units do not count shared UTF-8 lead bytes: {document}"
    );
}

#[test]
fn whitespace_is_source_content_not_normalised() {
    let old_lines = "// a                    \n".repeat(50);
    let new_lines = "// a\n".repeat(50);
    let old = format!("pub struct Same {{ pub value: u32 }}\n{old_lines}");
    let new = format!("pub struct Same {{ pub value: u32 }}\n{new_lines}");

    let repo = TestRepo::init();
    repo.write("old.rs", &old);
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &new);
    repo.commit("whitespace difference");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "whitespace counts as source content: {document}"
    );
}

#[test]
fn exactly_256_kib_is_fuzzy_eligible_and_one_byte_over_is_not() {
    let limit = 256 * 1024;
    let repo = TestRepo::init();
    repo.write("old.rs", &padded_comment_source(false, limit, "old"));
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &padded_comment_source(false, limit, "new"));
    repo.commit("at fuzzy limit");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);

    let repo = TestRepo::init();
    repo.write("old.rs", &padded_comment_source(false, limit + 1, "old"));
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &padded_comment_source(false, limit + 1, "new"));
    repo.commit("over fuzzy limit");
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "over 256 KiB is ineligible for fuzzy pairing: {document}"
    );
}

#[test]
fn tests_mode_omits_non_test_moves_and_keeps_test_gain_and_loss() {
    let non_test = exact_move_repo();
    let document = diff_json(&non_test, "tests", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        files(&document).is_empty(),
        "non-test move is absent: {document}"
    );

    let shared = format!("// {}\n", "shared".repeat(200));
    let repo = TestRepo::init();
    repo.write("old.rs", &format!("pub fn helper() {{}}\n{shared}"));
    repo.commit("base");
    repo.remove("old.rs");
    repo.write(
        "new.rs",
        &format!("#[test]\nfn moved_test() {{\n    assert!(true);\n}}\n{shared}"),
    );
    repo.commit("gained test");
    let document = diff_json(&repo, "tests", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", false);
    assert!(
        file_at(&document, "new.rs")["target"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("moved_test"),
        "{document}"
    );

    let shared = format!("// {}\n", "shared".repeat(200));
    let repo = TestRepo::init();
    repo.write(
        "old.rs",
        &format!("#[test]\nfn moved_test() {{\n    assert!(true);\n}}\n{shared}"),
    );
    repo.commit("base");
    repo.remove("old.rs");
    repo.write("new.rs", &format!("pub fn helper() {{}}\n{shared}"));
    repo.commit("lost test");
    let document = diff_json(&repo, "tests", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", false);
    assert!(
        file_at(&document, "new.rs")["base"]["projection"]["text"]
            .as_str()
            .unwrap()
            .contains("moved_test"),
        "{document}"
    );
}

#[test]
fn body_only_test_move_is_equal_in_tests_mode() {
    let repo = TestRepo::init();
    repo.write(
        "old.rs",
        "#[test]\nfn moved_test() {\n    assert!(true);\n}\n",
    );
    repo.commit("base");
    repo.git_ok(&["mv", "old.rs", "new.rs"]);
    repo.write(
        "new.rs",
        "#[test]\nfn moved_test() {\n    assert!(false);\n}\n",
    );
    repo.commit("body-only test");
    let document = diff_json(&repo, "tests", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);
    let text = run_text(&repo, "tests", "HEAD~1", "HEAD");
    assert!(
        !text.contains("@@"),
        "body-only test move has no hunk: {text}"
    );
}

#[test]
fn off_scope_candidates_count_toward_the_fuzzy_cap() {
    let repo = TestRepo::init();
    for index in 0..257 {
        repo.write(
            &format!("src/old_{index:04}.rs"),
            &format!("pub struct Old{index:04} {{ pub value: u32 }}\n"),
        );
    }
    repo.write("scope/old.rs", RUST_OLD);
    repo.commit("base");
    for index in 0..257 {
        fs::remove_file(repo.path().join(format!("src/old_{index:04}.rs")))
            .expect("remove old candidate");
        repo.write(
            &format!("src/new_{index:04}.rs"),
            &format!("pub struct Old{index:04} {{ pub value: u64 }}\n"),
        );
    }
    fs::remove_file(repo.path().join("scope/old.rs")).expect("remove scoped source");
    repo.write("scope/new.rs", RUST_BODY_EDIT);
    repo.commit("capped moves");

    let document = run_json(
        &repo,
        &[
            "diff", "--format", "json", "--mode", "types", "--path", "scope", "HEAD~1", "HEAD",
        ],
    );
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "the global candidate cap must run before filtering: {document}"
    );
    assert_eq!(files(&document).len(), 2, "{document}");
    assert_eq!(file_at(&document, "scope/old.rs")["status"], "deleted");
    assert_eq!(file_at(&document, "scope/new.rs")["status"], "added");
}

#[test]
fn side_cap_is_inclusive_at_256_and_disabled_at_257() {
    // 256 endpoints per side and exactly 4,096 eligible pairs: both inclusive
    // limits pass, so fuzzy runs and both sides pair.
    let repo = partitioned_side_cap_repo(false);
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(
        files(&document).len(),
        256,
        "256 raw endpoints must be eligible: {document}"
    );
    assert!(
        files(&document)
            .iter()
            .all(|file| file["status"] == "moved"),
        "exactly 4,096 eligible pairs must run: {document}"
    );

    // The extra oversized cross-language endpoints raise each side to 257 raw
    // leftovers but add no eligible pairs. Only the raw side cap can fire.
    let repo = partitioned_side_cap_repo(true);
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(
        files(&document).len(),
        514,
        "257 raw endpoints fall back: {document}"
    );
    assert!(!statuses(&document).contains(&"moved"));
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "deleted")
            .count(),
        257
    );
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "added")
            .count(),
        257
    );

    let repeated = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        serde_json::to_string(&repeated).unwrap(),
        "fallback must be deterministic"
    );
}

#[test]
fn selected_exact_pair_with_invalid_source_stays_fatal() {
    let repo = TestRepo::init();
    write_bytes(&repo, "old.rs", b"pub fn broken() { \xFF }\n");
    repo.commit("base");
    repo.git_ok(&["mv", "old.rs", "new.rs"]);
    repo.commit("move invalid source");

    let output = run(
        &repo,
        &[
            "diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "fatal output must be empty");
    assert!(
        !output.stderr.is_empty(),
        "a fatal error needs a diagnostic"
    );
}

#[cfg(unix)]
#[test]
fn unreadable_off_scope_fuzzy_candidate_is_not_read() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::init();
    repo.write("off.rs", RUST_OLD);
    repo.write("keep.rs", RUST_SMALL);
    repo.commit("base");
    fs::remove_file(repo.path().join("off.rs")).expect("remove off.rs");
    repo.write("new_off.rs", RUST_TYPE_EDIT);
    repo.write("keep.rs", RUST_TYPE_EDIT);
    let candidate = repo.path().join("new_off.rs");
    let mut permissions = fs::metadata(&candidate)
        .expect("candidate metadata")
        .permissions();
    permissions.set_mode(0o000);
    if fs::set_permissions(&candidate, permissions).is_err() {
        eprintln!("skipping unreadable off-scope test: chmod failed");
        return;
    }

    let output = run(
        &repo,
        &[
            "diff",
            "--format",
            "json",
            "--mode",
            "types",
            "--path",
            "keep.rs",
            "HEAD",
            ":worktree",
        ],
    );
    let mut permissions = fs::metadata(&candidate)
        .expect("candidate metadata")
        .permissions();
    permissions.set_mode(0o644);
    let _ = fs::set_permissions(&candidate, permissions);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let document: Value = serde_json::from_str(&stdout(&output)).expect("JSON");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 1, "{document}");
    assert_eq!(file_at(&document, "keep.rs")["status"], "modified");
}

#[test]
fn moved_json_document_validates_against_the_v2_schema() {
    let repo = exact_move_repo();
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_moved(file_at(&document, "new.rs"), "old.rs", "new.rs", true);

    let schema: Value = serde_json::from_str(&doc("schema/codect.diff.v2.json"))
        .expect("codect.diff.v2.json must exist and be valid JSON");
    support::schema::validate(&schema, &document).expect("moved document validates as v2");
}

#[test]
fn non_moved_events_never_carry_old_path() {
    let repo = TestRepo::init();
    repo.write("modified.rs", RUST_SMALL);
    repo.write(
        "deleted.rs",
        &format!(
            "pub struct Removed {{ pub value: u32 }}\n// {}\n",
            "d".repeat(1000)
        ),
    );
    repo.commit("base");
    repo.write("modified.rs", RUST_TYPE_EDIT);
    repo.remove("deleted.rs");
    repo.write(
        "added.rs",
        &format!(
            "pub struct Added {{ pub value: u32 }}\n// {}\n",
            "a".repeat(1000)
        ),
    );
    repo.commit("three statuses");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    for file in files(&document) {
        assert_no_old_path(file);
        assert_eq!(file["equal"], false, "{file}");
    }
    assert_eq!(
        statuses(&document),
        ["added", "deleted", "modified"],
        "{document}"
    );
}

#[test]
fn side_cap_counts_raw_leftovers_before_language_filters() {
    let repo = TestRepo::init();
    for index in 0..256 {
        repo.write(
            &format!("src/old_{index:04}.rs"),
            &rust_candidate_source(&format!("Old{index:04}"), "u32"),
        );
    }
    // This extra endpoint makes 257 raw old paths and 257 raw new paths.
    // Only 256 pairs would remain after language filtering, but the side cap
    // applies to the exact leftovers before that filtering, so fuzzy is off.
    repo.write("src/old_lang.rs", &rust_candidate_source("Cross", "u32"));
    repo.commit("base");
    for index in 0..256 {
        fs::remove_file(repo.path().join(format!("src/old_{index:04}.rs")))
            .expect("remove old candidate");
        repo.write(
            &format!("src/new_{index:04}.rs"),
            &rust_candidate_source(&format!("Old{index:04}"), "u64"),
        );
    }
    fs::remove_file(repo.path().join("src/old_lang.rs")).expect("remove language candidate");
    repo.write("src/new_lang.py", &python_source("Cross"));
    repo.commit("raw endpoint cap");

    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert!(
        !statuses(&document).contains(&"moved"),
        "the raw 256-path side cap must run before language filtering: {document}"
    );
    assert_eq!(files(&document).len(), 514, "{document}");
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "deleted")
            .count(),
        257,
        "{document}"
    );
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "added")
            .count(),
        257,
        "{document}"
    );
}

#[test]
fn pair_cap_counts_only_eligible_pairs_after_filters() {
    // 65 endpoints per side: raw cross-product 4,225, language-filtered
    // eligible pairs 63^2 + 2^2 = 3,973. The side cap passes, and the pair cap
    // must be evaluated after language filtering.
    let repo = py_rust_pair_repo(63, 2);
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 65, "{document}");
    assert!(
        files(&document)
            .iter()
            .all(|file| file["status"] == "moved"),
        "4,225 raw combinations, 3,973 eligible pairs, 65 moves: {document}"
    );
}

#[test]
fn pair_cap_at_4096_eligible_pairs_runs() {
    // 64 endpoints per side, all Python, so exactly 4,096 eligible pairs.
    let repo = py_rust_pair_repo(64, 0);
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 64, "{document}");
    assert!(
        files(&document)
            .iter()
            .all(|file| file["status"] == "moved"),
        "exactly 4,096 eligible pairs must run: {document}"
    );
}

#[test]
fn pair_cap_4097_eligible_pairs_disables_fuzzy() {
    // 65 endpoints per side: Python 64x64 = 4,096 plus Rust 1x1 = 1, for
    // 4,097 eligible pairs. The side cap passes; the pair cap disables fuzzy.
    let repo = py_rust_pair_repo(64, 1);
    let document = diff_json(&repo, "types", "HEAD~1", "HEAD");
    assert_schema_v2(&document);
    assert_eq!(files(&document).len(), 130, "{document}");
    assert!(!statuses(&document).contains(&"moved"));
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "deleted")
            .count(),
        65,
        "{document}"
    );
    assert_eq!(
        statuses(&document)
            .iter()
            .filter(|status| **status == "added")
            .count(),
        65,
        "{document}"
    );
}
