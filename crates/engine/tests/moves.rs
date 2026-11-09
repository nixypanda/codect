// Move-detection behavior over real temporary Git repositories.
//
// Detection is source-content based and independent of the requested
// projection mode: a body-only edit can still be one move whose focused
// projections are equal, while a move with two empty requested-mode
// projections is omitted.

mod support;

use base::{Area, FileDiff, FileOutlineDiff, ProjectionMode, RepoPath, Selection, SelectionGroup};
use engine::{DiffBase, Engine, EngineError};
use support::TestRepo;

const SOURCE: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

const TEST_SOURCE: &str = "\
pub struct Marker;

#[test]
fn marker() {
    assert!(true);
}
";

fn engine(repo: &TestRepo) -> Engine {
    Engine::discover(repo.path()).expect("discover repository")
}

fn all() -> Selection {
    Selection::all()
}

fn path_selection(paths: &[&str]) -> Selection {
    let groups = paths
        .iter()
        .map(|raw| SelectionGroup::Path(RepoPath::new(*raw).expect("valid path")))
        .collect();
    Selection::new(groups)
}

fn area_selection(name: &str, paths: &[&str]) -> Selection {
    let paths: Vec<RepoPath> = paths
        .iter()
        .map(|raw| RepoPath::new(*raw).expect("valid path"))
        .collect();
    let area = Area::new(name, paths).expect("valid area");
    Selection::new(vec![SelectionGroup::Area(area)])
}

fn moved_paths(diffs: &[FileDiff]) -> Vec<(String, String)> {
    diffs
        .iter()
        .filter_map(|diff| match diff {
            FileDiff::Moved { old, new } => Some((old.path().to_string(), new.path().to_string())),
            _ => None,
        })
        .collect()
}

fn outline_moved_paths(diffs: &[FileOutlineDiff]) -> Vec<(String, String)> {
    diffs
        .iter()
        .filter_map(|diff| match diff {
            FileOutlineDiff::Moved { old, new } => {
                Some((old.path().to_string(), new.path().to_string()))
            }
            _ => None,
        })
        .collect()
}

fn statuses(diffs: &[FileDiff]) -> Vec<&'static str> {
    diffs
        .iter()
        .map(|diff| match diff {
            FileDiff::Added { .. } => "added",
            FileDiff::Deleted { .. } => "deleted",
            FileDiff::Modified { .. } => "modified",
            FileDiff::Moved { .. } => "moved",
        })
        .collect()
}

fn outline_statuses(diffs: &[FileOutlineDiff]) -> Vec<&'static str> {
    diffs
        .iter()
        .map(|diff| match diff {
            FileOutlineDiff::Added { .. } => "added",
            FileOutlineDiff::Deleted { .. } => "deleted",
            FileOutlineDiff::Modified { .. } => "modified",
            FileOutlineDiff::Moved { .. } => "moved",
        })
        .collect()
}

#[test]
fn exact_move_is_one_event_in_all_three_diff_apis() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    let target = repo.commit("move");

    let engine = engine(&repo);
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let projections = engine
            .diff(&base, &target, mode, &all(), DiffBase::Given)
            .expect("projection diff");
        assert_eq!(
            moved_paths(&projections),
            [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
        );
        assert_eq!(projections[0].path().to_string(), "src/new.rs");
        assert_eq!(
            projections[0].old_path().map(ToString::to_string),
            Some("src/old.rs".to_owned())
        );
        assert_eq!(
            projections[0].new_path().map(ToString::to_string),
            Some("src/new.rs".to_owned())
        );

        let outlines = engine
            .diff_outlines(&base, &target, mode, &all(), DiffBase::Given)
            .expect("outline diff");
        assert_eq!(
            outline_moved_paths(&outlines.files),
            [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
        );

        let snapshots = engine
            .diff_snapshot_outlines(&base, &target, mode, &all(), DiffBase::Given)
            .expect("snapshot outline diff");
        assert_eq!(
            outline_moved_paths(&snapshots.files),
            [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
        );
    }

    let tests = engine
        .diff(
            &base,
            &target,
            ProjectionMode::Tests,
            &all(),
            DiffBase::Given,
        )
        .expect("tests diff");
    assert!(
        tests.is_empty(),
        "a non-test file has two empty Tests projections and is omitted"
    );
}

#[test]
fn body_only_edit_inside_a_move_has_equal_projections_and_no_hunks() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", "pub fn value() -> u8 {\n    1\n}\n");
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.write("src/new.rs", "pub fn value() -> u8 {\n    2\n}\n");
    let target = repo.commit("body edit");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Signatures,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(statuses(&diffs), ["moved"]);
    match &diffs[0] {
        FileDiff::Moved { old, new } => {
            assert_eq!(old.canonical_text(), "pub fn value() -> u8;\n");
            assert_eq!(old.canonical_text(), new.canonical_text());
        }
        other => panic!("expected a move, got {other:?}"),
    }
}

#[test]
fn a_signature_edit_inside_a_move_keeps_the_move_and_changes_the_projection() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", "pub fn value() -> u8 {\n    1\n}\n");
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.write("src/new.rs", "pub fn value() -> u16 {\n    1\n}\n");
    let target = repo.commit("signature edit");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Signatures,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(statuses(&diffs), ["moved"]);
    match &diffs[0] {
        FileDiff::Moved { old, new } => {
            assert_eq!(old.canonical_text(), "pub fn value() -> u8;\n");
            assert_eq!(new.canonical_text(), "pub fn value() -> u16;\n");
        }
        other => panic!("expected a move, got {other:?}"),
    }
}

#[test]
fn tests_mode_shows_a_moved_test_file_but_not_a_non_test_file() {
    let repo = TestRepo::init();
    repo.write("src/test_old.rs", TEST_SOURCE);
    repo.write("src/plain.rs", SOURCE);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/test_old.rs", "src/test_new.rs"]);
    repo.git_ok(&["mv", "src/plain.rs", "src/plain_new.rs"]);
    let target = repo.commit("moves");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Tests,
            &all(),
            DiffBase::Given,
        )
        .expect("tests diff");

    assert_eq!(
        moved_paths(&diffs),
        [("src/test_old.rs".to_owned(), "src/test_new.rs".to_owned())],
        "only the file with a test declaration is visible in Tests"
    );
}

#[test]
fn a_move_that_gains_or_loses_its_last_test_stays_visible() {
    let lose_common = (0..40)
        .map(|index| format!("pub const LOSE_VALUE_{index}: u8 = {index};\n"))
        .collect::<String>();
    let gain_common = (0..40)
        .map(|index| format!("pub const GAIN_VALUE_{index}: u16 = {index};\n"))
        .collect::<String>();

    let lose_with_test =
        format!("{lose_common}\n#[test]\nfn last_test() {{\n    assert!(true);\n}}\n");
    let lose_without_test = format!("{lose_common}\n");
    let gain_with_test =
        format!("{gain_common}\n#[test]\nfn last_test() {{\n    assert!(true);\n}}\n");
    let gain_without_test = format!("{gain_common}\n");

    let repo = TestRepo::init();
    repo.write("src/lose_old.rs", &lose_with_test);
    repo.write("src/gain_old.rs", &gain_without_test);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/lose_old.rs", "src/lose_new.rs"]);
    repo.git_ok(&["mv", "src/gain_old.rs", "src/gain_new.rs"]);
    repo.write("src/lose_new.rs", &lose_without_test);
    repo.write("src/gain_new.rs", &gain_with_test);
    let target = repo.commit("test churn");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Tests,
            &all(),
            DiffBase::Given,
        )
        .expect("tests diff");

    assert_eq!(
        moved_paths(&diffs),
        [
            ("src/gain_old.rs".to_owned(), "src/gain_new.rs".to_owned()),
            ("src/lose_old.rs".to_owned(), "src/lose_new.rs".to_owned()),
        ]
    );
    match &diffs[0] {
        FileDiff::Moved { old, new } => {
            assert!(old.canonical_text().is_empty());
            assert!(!new.canonical_text().is_empty());
        }
        other => panic!("expected a move, got {other:?}"),
    }
    match &diffs[1] {
        FileDiff::Moved { old, new } => {
            assert!(!old.canonical_text().is_empty());
            assert!(new.canonical_text().is_empty());
        }
        other => panic!("expected a move, got {other:?}"),
    }
}

#[test]
fn same_projection_but_unrelated_sources_do_not_pair() {
    let old_body = "alpha ".repeat(400);
    let new_body = "bravo ".repeat(400);
    let old =
        format!("pub struct Same;\n\npub fn value() -> u8 {{\n    // {old_body}\n    1\n}}\n");
    let new =
        format!("pub struct Same;\n\npub fn value() -> u8 {{\n    // {new_body}\n    2\n}}\n");

    let repo = TestRepo::init();
    repo.write("src/old.rs", &old);
    let base = repo.commit("base");
    repo.remove("src/old.rs");
    repo.write("src/new.rs", &new);
    let target = repo.commit("replace");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Signatures,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(
        statuses(&diffs),
        ["added", "deleted"],
        "events sort by target path; src/new.rs precedes src/old.rs"
    );
    assert!(moved_paths(&diffs).is_empty());
}

#[test]
fn ambiguous_identical_sources_pair_by_raw_path_bytes() {
    let repo = TestRepo::init();
    repo.write("src/a.rs", SOURCE);
    repo.write("src/b.rs", SOURCE);
    let base = repo.commit("base");
    repo.remove("src/a.rs");
    repo.remove("src/b.rs");
    repo.write("src/c.rs", SOURCE);
    repo.write("src/d.rs", SOURCE);
    let target = repo.commit("replace");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(
        moved_paths(&diffs),
        [
            ("src/a.rs".to_owned(), "src/c.rs".to_owned()),
            ("src/b.rs".to_owned(), "src/d.rs".to_owned()),
        ]
    );
}

#[test]
fn a_copy_remains_an_addition() {
    let repo = TestRepo::init();
    repo.write("src/a.rs", SOURCE);
    let base = repo.commit("base");
    repo.write("src/copy.rs", SOURCE);
    let target = repo.commit("copy");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(statuses(&diffs), ["added"]);
    assert_eq!(diffs[0].path().to_string(), "src/copy.rs");
}

#[test]
fn a_move_is_returned_for_an_old_only_or_new_only_scope() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    repo.write("src/keep.rs", SOURCE);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    let target = repo.commit("move");

    let engine = engine(&repo);
    for selection in [
        path_selection(&["src/old.rs"]),
        path_selection(&["src/new.rs"]),
        area_selection("src", &["src"]),
    ] {
        let diffs = engine
            .diff(
                &base,
                &target,
                ProjectionMode::Types,
                &selection,
                DiffBase::Given,
            )
            .expect("scoped diff");
        assert_eq!(
            moved_paths(&diffs),
            [("src/old.rs".to_owned(), "src/new.rs".to_owned())],
            "either path selects the whole move"
        );
        assert_eq!(diffs[0].path().to_string(), "src/new.rs");
    }
}

#[test]
fn a_narrow_scope_cannot_change_pairing() {
    let repo = TestRepo::init();
    repo.write("src/a.rs", SOURCE);
    repo.write("src/b.rs", SOURCE);
    let base = repo.commit("base");
    repo.remove("src/a.rs");
    repo.remove("src/b.rs");
    repo.write("src/c.rs", SOURCE);
    repo.write("src/d.rs", SOURCE);
    let target = repo.commit("replace");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &path_selection(&["src/a.rs"]),
            DiffBase::Given,
        )
        .expect("scoped diff");

    assert_eq!(
        moved_paths(&diffs),
        [("src/a.rs".to_owned(), "src/c.rs".to_owned())],
        "the complete snapshot resolves ambiguity before the scope is applied"
    );
}

#[test]
fn unstaged_rename_is_detected_against_the_worktree() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    repo.write("src/keep.rs", SOURCE);
    repo.commit("base");

    std::fs::rename(
        repo.path().join("src/old.rs"),
        repo.path().join("src/new.rs"),
    )
    .expect("rename on disk without staging");

    let engine = engine(&repo);
    let worktree = engine
        .diff_snapshot_outlines(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("worktree diff");
    assert_eq!(
        outline_moved_paths(&worktree.files),
        [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
    );

    let projections = engine
        .diff(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("worktree projection diff");
    assert_eq!(
        moved_paths(&projections),
        [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
    );

    let index = engine
        .diff_snapshot_outlines(
            "HEAD",
            ":index",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("index diff");
    assert!(
        index.files.is_empty(),
        "the unstaged move is still an unchanged index entry"
    );
}

#[test]
fn staged_rename_is_detected_against_the_index() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);

    let files = engine(&repo)
        .diff_snapshot_outlines(
            "HEAD",
            ":index",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("index diff")
        .files;
    assert_eq!(
        outline_moved_paths(&files),
        [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
    );
}

#[test]
fn reversal_swaps_the_move_endpoints() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    let target = repo.commit("move");

    let forward = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("forward");
    let reverse = engine(&repo)
        .diff(
            &target,
            &base,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("reverse");

    assert_eq!(
        moved_paths(&forward),
        [("src/old.rs".to_owned(), "src/new.rs".to_owned())]
    );
    assert_eq!(
        moved_paths(&reverse),
        [("src/new.rs".to_owned(), "src/old.rs".to_owned())]
    );
}

#[test]
fn an_exact_move_with_two_empty_projections_is_omitted() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", "// no declarations\n");
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    let target = repo.commit("move");

    let engine = engine(&repo);
    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        assert!(
            engine
                .diff(&base, &target, mode, &all(), DiffBase::Given)
                .expect("diff")
                .is_empty(),
            "{mode:?} must omit a move with two empty projections"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_move_with_non_utf8_paths_is_detected_and_sorted_by_raw_bytes() {
    let repo = TestRepo::init();
    repo.write("src/base.rs", SOURCE);
    repo.commit("base");
    let blob = repo
        .git_ok(&["rev-parse", "HEAD:src/base.rs"])
        .trim()
        .to_owned();

    let old_path = b"src/\xFF/old.rs";
    let new_path = b"src/\xFE/new.rs";
    let old_cacheinfo = {
        let mut value = format!("100644,{blob},").into_bytes();
        value.extend_from_slice(old_path);
        value
    };
    repo.git_bytes_ok(&[
        b"update-index",
        b"--add",
        b"--cacheinfo",
        old_cacheinfo.as_slice(),
    ]);
    repo.git_ok(&["commit", "-q", "-m", "old invalid path"]);
    let base = repo.head_id();

    let new_cacheinfo = {
        let mut value = format!("100644,{blob},").into_bytes();
        value.extend_from_slice(new_path);
        value
    };
    repo.git_bytes_ok(&[
        b"update-index",
        b"--add",
        b"--cacheinfo",
        new_cacheinfo.as_slice(),
    ]);
    repo.git_bytes_ok(&[b"update-index", b"--force-remove", old_path]);
    repo.git_ok(&["commit", "-q", "-m", "new invalid path"]);
    let target = repo.head_id();

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(statuses(&diffs), ["moved"]);
    let FileDiff::Moved { old, new } = &diffs[0] else {
        panic!("expected a move, got {:?}", diffs[0]);
    };
    assert_eq!(old.path().as_bytes(), old_path);
    assert_eq!(new.path().as_bytes(), new_path);
}

#[test]
fn a_directory_replacing_a_tracked_file_is_not_present() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    repo.commit("base");

    std::fs::remove_file(repo.path().join("src/old.rs")).expect("remove file");
    std::fs::create_dir(repo.path().join("src/old.rs")).expect("replace with directory");

    let files = engine(&repo)
        .diff_snapshot_outlines(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("worktree diff")
        .files;

    assert_eq!(
        outline_statuses(&files),
        ["deleted"],
        "a directory is not a supported source file, so the old path is deleted"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_replacing_a_tracked_file_is_not_present() {
    use std::os::unix::fs::symlink;

    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    repo.write("src/target.rs", SOURCE);
    repo.commit("base");

    std::fs::remove_file(repo.path().join("src/old.rs")).expect("remove file");
    symlink("target.rs", repo.path().join("src/old.rs")).expect("replace with symlink");

    let files = engine(&repo)
        .diff_snapshot_outlines(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("worktree diff")
        .files;

    assert_eq!(outline_statuses(&files), ["deleted"]);
}

#[test]
fn a_large_exact_worktree_move_is_detected_without_fuzzy_scoring() {
    let mut source = String::new();
    while source.len() <= 256 * 1024 {
        source.push_str("// padding padding padding padding padding padding\n");
    }
    source.push_str("pub struct Large;\n");

    let repo = TestRepo::init();
    repo.write("src/old.rs", &source);
    repo.commit("base");

    std::fs::rename(
        repo.path().join("src/old.rs"),
        repo.path().join("src/new.rs"),
    )
    .expect("rename large file on disk");

    let diffs = engine(&repo)
        .diff(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("worktree diff");

    assert_eq!(
        moved_paths(&diffs),
        [("src/old.rs".to_owned(), "src/new.rs".to_owned())],
        "exact pairing must still run above the fuzzy byte cap"
    );
}

#[test]
fn unchanged_same_path_blobs_are_skipped_without_reading_them() {
    let repo = TestRepo::init();
    std::fs::write(repo.path().join("bad.rs"), b"pub struct Broken;\n\xFF")
        .expect("write invalid UTF-8 fixture");
    repo.write("src/keep.rs", SOURCE);
    let base = repo.commit("base");
    repo.write("src/changed.rs", "pub struct Changed;\n");
    let target = repo.commit("change another file");

    let engine = engine(&repo);
    let projections = engine
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("unchanged invalid UTF-8 is never projected");
    assert_eq!(
        projections
            .iter()
            .map(|diff| diff.path().to_string())
            .collect::<Vec<_>>(),
        ["src/changed.rs"]
    );

    let outlines = engine
        .diff_outlines(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("outline fast path");
    assert_eq!(
        outlines
            .files
            .iter()
            .map(|diff| diff.path().to_string())
            .collect::<Vec<_>>(),
        ["src/changed.rs"]
    );

    let snapshots = engine
        .diff_snapshot_outlines(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("snapshot outline fast path");
    assert_eq!(
        snapshots
            .files
            .iter()
            .map(|diff| diff.path().to_string())
            .collect::<Vec<_>>(),
        ["src/changed.rs"]
    );
}

#[test]
fn too_many_unpaired_paths_skip_fuzzy_pairing_entirely() {
    let repo = TestRepo::init();
    for index in 0..257 {
        repo.write(
            &format!("old/file_{index:03}.rs"),
            &format!("pub struct Old{index};\n"),
        );
    }
    let base = repo.commit("base");
    for index in 0..257 {
        repo.remove(&format!("old/file_{index:03}.rs"));
        repo.write(
            &format!("new/file_{index:03}.rs"),
            &format!("pub struct New{index};\n"),
        );
    }
    let target = repo.commit("replace");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(diffs.len(), 514);
    assert!(
        moved_paths(&diffs).is_empty(),
        "more than 256 unpaired paths per side is a deterministic fallback"
    );
}

#[test]
fn too_many_eligible_pairs_skip_fuzzy_pairing_entirely() {
    let repo = TestRepo::init();
    for index in 0..65 {
        repo.write(
            &format!("old/file_{index:02}.rs"),
            &format!("pub struct Old{index:02};\n"),
        );
    }
    let base = repo.commit("base");
    for index in 0..65 {
        repo.remove(&format!("old/file_{index:02}.rs"));
        repo.write(
            &format!("new/file_{index:02}.rs"),
            &format!("pub struct New{index:02};\n"),
        );
    }
    let target = repo.commit("replace");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");

    assert_eq!(diffs.len(), 130);
    assert!(
        moved_paths(&diffs).is_empty(),
        "4,225 eligible pairs exceeds the 4,096 pair cap"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_off_scope_worktree_candidate_is_nonfatal_but_selected_is_fatal() {
    use std::os::unix::fs::PermissionsExt;

    let repo = TestRepo::init();
    repo.write("src/old.rs", SOURCE);
    repo.write("src/keep.rs", SOURCE);
    repo.commit("base");

    std::fs::rename(
        repo.path().join("src/old.rs"),
        repo.path().join("src/new.rs"),
    )
    .expect("rename on disk");
    let unreadable = repo.path().join("src/new.rs");
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000))
        .expect("make unreadable");
    if std::fs::read(&unreadable).is_ok() {
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644))
            .expect("restore permissions");
        return;
    }

    let engine = engine(&repo);
    let scoped = engine
        .diff(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &path_selection(&["src/keep.rs"]),
            DiffBase::Given,
        )
        .expect("off-scope discovery failure must not abort the comparison");
    assert!(scoped.is_empty());

    let error = engine
        .diff(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect_err("a selected unreadable worktree source remains fatal");
    assert!(
        matches!(error, EngineError::WorktreeRead { .. }),
        "got {error:?}"
    );

    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o644))
        .expect("restore permissions");
}
