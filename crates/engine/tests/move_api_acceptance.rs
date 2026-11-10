// Independent parity checks for the three engine diff APIs:
//
// - `diff` returns projection values;
// - `diff_outlines` returns commit outlines;
// - `diff_snapshot_outlines` returns snapshot outlines.
//
// A move must look the same through all three. This file also checks the
// unchanged same-path blob fast path by deleting the blob object from a
// temporary repository: a read would fail, while a blob-id skip succeeds.

mod support;

use std::fs;

use base::{FileDiff, FileOutlineDiff, ProjectionMode, RepoPath, Selection, SelectionGroup};
use engine::{DiffBase, Engine};
use support::TestRepo;

const TEST_OLD: &str = "\
pub struct Marker;

#[test]
fn it_works() {
    assert!(true);
}
";

const TEST_BODY: &str = "\
pub struct Marker;

#[test]
fn it_works() {
    assert!(false);
}
";

const SIG_OLD: &str = "\
pub struct Value {
    pub n: u8,
}
";

const SIG_NEW: &str = "\
pub struct Value {
    pub n: u16,
}
";

const SIG_FEATURE: &str = "\
pub struct Value {
    pub n: u32,
}
";

const PLAIN: &str = "pub struct Plain;\n";

const KEEP: &str = "\
pub struct Keep {
    pub n: u32,
}
";

#[derive(Clone, Debug, Eq, PartialEq)]
struct Event {
    status: &'static str,
    old_path: Option<String>,
    new_path: Option<String>,
    equal: Option<bool>,
}

fn engine(repo: &TestRepo) -> Engine {
    Engine::discover(repo.path()).expect("discover repository")
}

fn all() -> Selection {
    Selection::all()
}

fn path_selection(raw: &str) -> Selection {
    Selection::new(vec![SelectionGroup::Path(
        RepoPath::new(raw).expect("valid path"),
    )])
}

fn moved(old: &str, new: &str, equal: bool) -> Event {
    Event {
        status: "moved",
        old_path: Some(old.to_owned()),
        new_path: Some(new.to_owned()),
        equal: Some(equal),
    }
}

fn modified(path: &str) -> Event {
    Event {
        status: "modified",
        old_path: Some(path.to_owned()),
        new_path: Some(path.to_owned()),
        equal: None,
    }
}

fn projection_events(diffs: &[FileDiff]) -> Vec<Event> {
    diffs
        .iter()
        .map(|diff| match diff {
            FileDiff::Added { new } => Event {
                status: "added",
                old_path: None,
                new_path: Some(new.path().to_string()),
                equal: None,
            },
            FileDiff::Deleted { old } => Event {
                status: "deleted",
                old_path: Some(old.path().to_string()),
                new_path: None,
                equal: None,
            },
            FileDiff::Modified { old, new } => Event {
                status: "modified",
                old_path: Some(old.path().to_string()),
                new_path: Some(new.path().to_string()),
                equal: None,
            },
            FileDiff::Moved { old, new } => Event {
                status: "moved",
                old_path: Some(old.path().to_string()),
                new_path: Some(new.path().to_string()),
                equal: Some(old.canonical_text() == new.canonical_text()),
            },
        })
        .collect()
}

fn outline_events(diffs: &[FileOutlineDiff]) -> Vec<Event> {
    diffs
        .iter()
        .map(|diff| match diff {
            FileOutlineDiff::Added { new } => Event {
                status: "added",
                old_path: None,
                new_path: Some(new.path().to_string()),
                equal: None,
            },
            FileOutlineDiff::Deleted { old } => Event {
                status: "deleted",
                old_path: Some(old.path().to_string()),
                new_path: None,
                equal: None,
            },
            FileOutlineDiff::Modified { old, new } => Event {
                status: "modified",
                old_path: Some(old.path().to_string()),
                new_path: Some(new.path().to_string()),
                equal: None,
            },
            FileOutlineDiff::Moved { old, new } => Event {
                status: "moved",
                old_path: Some(old.path().to_string()),
                new_path: Some(new.path().to_string()),
                equal: Some(old.projection.canonical_text() == new.projection.canonical_text()),
            },
        })
        .collect()
}

fn assert_three_way(
    engine: &Engine,
    base: &str,
    target: &str,
    mode: ProjectionMode,
    selection: &Selection,
    base_mode: DiffBase,
) -> Vec<Event> {
    let projections = engine
        .diff(base, target, mode, selection, base_mode)
        .unwrap_or_else(|error| panic!("diff failed for {mode:?} {base}..{target}: {error:?}"));
    let outlines = engine
        .diff_outlines(base, target, mode, selection, base_mode)
        .unwrap_or_else(|error| {
            panic!("diff_outlines failed for {mode:?} {base}..{target}: {error:?}")
        });
    let snapshots = engine
        .diff_snapshot_outlines(base, target, mode, selection, base_mode)
        .unwrap_or_else(|error| {
            panic!("diff_snapshot_outlines failed for {mode:?} {base}..{target}: {error:?}")
        });

    let projection_event_list = projection_events(&projections);
    let outline_event_list = outline_events(&outlines.files);
    let snapshot_event_list = outline_events(&snapshots.files);
    assert_eq!(
        projection_event_list, outline_event_list,
        "diff and diff_outlines disagree for {mode:?} {base}..{target}"
    );
    assert_eq!(
        projection_event_list, snapshot_event_list,
        "diff and diff_snapshot_outlines disagree for {mode:?} {base}..{target}"
    );
    projection_event_list
}

fn assert_snapshot_way(
    engine: &Engine,
    base: &str,
    target: &str,
    mode: ProjectionMode,
) -> Vec<Event> {
    let projections = engine
        .diff(base, target, mode, &all(), DiffBase::Given)
        .unwrap_or_else(|error| panic!("diff failed for {mode:?} {base}..{target}: {error:?}"));
    let snapshots = engine
        .diff_snapshot_outlines(base, target, mode, &all(), DiffBase::Given)
        .unwrap_or_else(|error| {
            panic!("diff_snapshot_outlines failed for {mode:?} {base}..{target}: {error:?}")
        });
    let projection_event_list = projection_events(&projections);
    let snapshot_event_list = outline_events(&snapshots.files);
    assert_eq!(
        projection_event_list, snapshot_event_list,
        "diff and diff_snapshot_outlines disagree for {mode:?} {base}..{target}"
    );
    projection_event_list
}

fn common_constants(prefix: &str) -> String {
    (0..40)
        .map(|index| format!("pub const {prefix}_{index}: u8 = {index};\n"))
        .collect()
}

#[test]
fn exact_move_parity_for_all_modes_and_old_or_new_selection() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", TEST_OLD);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    let target = repo.commit("move");
    let engine = engine(&repo);

    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        for selection in [
            all(),
            path_selection("src/old.rs"),
            path_selection("src/new.rs"),
        ] {
            let events =
                assert_three_way(&engine, &base, &target, mode, &selection, DiffBase::Given);
            assert_eq!(
                events,
                [moved("src/old.rs", "src/new.rs", true)],
                "{mode:?} selection must retain the whole move"
            );
        }
    }
}

#[test]
fn exact_body_and_signature_moves_agree_on_all_three_apis() {
    let repo = TestRepo::init();
    repo.write("src/body_old.rs", TEST_OLD);
    repo.write("src/sig_old.rs", SIG_OLD);
    repo.write("src/plain_old.rs", PLAIN);
    let base = repo.commit("base");

    repo.git_ok(&["mv", "src/body_old.rs", "src/body_new.rs"]);
    repo.write("src/body_new.rs", TEST_BODY);
    repo.git_ok(&["mv", "src/sig_old.rs", "src/sig_new.rs"]);
    repo.write("src/sig_new.rs", SIG_NEW);
    repo.git_ok(&["mv", "src/plain_old.rs", "src/plain_new.rs"]);
    let target = repo.commit("edited and exact moves");
    let engine = engine(&repo);

    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let events = assert_three_way(&engine, &base, &target, mode, &all(), DiffBase::Given);
        assert_eq!(
            events,
            [
                moved("src/body_old.rs", "src/body_new.rs", true),
                moved("src/plain_old.rs", "src/plain_new.rs", true),
                moved("src/sig_old.rs", "src/sig_new.rs", false),
            ],
            "{mode:?} move order and equality"
        );

        for selection in [
            path_selection("src/body_old.rs"),
            path_selection("src/body_new.rs"),
        ] {
            assert_eq!(
                assert_three_way(&engine, &base, &target, mode, &selection, DiffBase::Given,),
                [moved("src/body_old.rs", "src/body_new.rs", true)],
                "{mode:?} body-only move selected by either endpoint"
            );
        }
    }

    assert_eq!(
        assert_three_way(
            &engine,
            &base,
            &target,
            ProjectionMode::Tests,
            &all(),
            DiffBase::Given,
        ),
        [moved("src/body_old.rs", "src/body_new.rs", true)],
        "Tests keeps the moved test file, omits the non-test signature and exact moves"
    );
}

#[test]
fn tests_gain_and_loss_parity_across_apis() {
    let loss_common = common_constants("LOSS");
    let gain_common = common_constants("GAIN");
    let test_source = "#[test]\nfn last_test() {\n    assert!(true);\n}\n";

    let repo = TestRepo::init();
    repo.write("src/loss_old.rs", &format!("{loss_common}\n{test_source}"));
    repo.write("src/gain_old.rs", &gain_common);
    let base = repo.commit("base");

    repo.git_ok(&["mv", "src/loss_old.rs", "src/loss_new.rs"]);
    repo.write("src/loss_new.rs", &loss_common);
    repo.git_ok(&["mv", "src/gain_old.rs", "src/gain_new.rs"]);
    repo.write("src/gain_new.rs", &format!("{gain_common}\n{test_source}"));
    let target = repo.commit("test gain and loss");
    let engine = engine(&repo);

    let events = assert_three_way(
        &engine,
        &base,
        &target,
        ProjectionMode::Tests,
        &all(),
        DiffBase::Given,
    );
    assert_eq!(
        events,
        [
            moved("src/gain_old.rs", "src/gain_new.rs", false),
            moved("src/loss_old.rs", "src/loss_new.rs", false),
        ],
        "Test-only moves sort by target path and have one non-empty side"
    );
}

#[test]
fn merge_base_move_parity_across_apis() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", SIG_OLD);
    let fork = repo.commit("fork");
    repo.git_ok(&["branch", "feature"]);
    repo.write("src/old.rs", SIG_NEW);
    let main = repo.commit("main change");
    repo.git_ok(&["checkout", "-q", "feature"]);
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.write("src/new.rs", SIG_FEATURE);
    let feature = repo.commit("feature move");

    let engine = engine(&repo);
    let projections = engine
        .diff(
            &main,
            "feature",
            ProjectionMode::Types,
            &all(),
            DiffBase::MergeBase,
        )
        .expect("merge-base diff");
    let outlines = engine
        .diff_outlines(
            &main,
            "feature",
            ProjectionMode::Types,
            &all(),
            DiffBase::MergeBase,
        )
        .expect("merge-base outlines");
    let snapshots = engine
        .diff_snapshot_outlines(
            &main,
            "feature",
            ProjectionMode::Types,
            &all(),
            DiffBase::MergeBase,
        )
        .expect("merge-base snapshots");

    let expected = [moved("src/old.rs", "src/new.rs", false)];
    assert_eq!(projection_events(&projections), expected);
    assert_eq!(outline_events(&outlines.files), expected);
    assert_eq!(outline_events(&snapshots.files), expected);
    assert_eq!(outlines.base_id.to_string(), fork);
    assert_eq!(snapshots.base_id, fork);
    assert_eq!(outlines.target_id.to_string(), feature);
    assert_eq!(snapshots.target_id, feature);
}

#[test]
fn index_and_worktree_move_parity_for_snapshot_apis() {
    let repo = TestRepo::init();
    repo.write("src/old.rs", TEST_OLD);
    repo.commit("base");
    repo.git_ok(&["mv", "src/old.rs", "src/new.rs"]);
    repo.write("src/new.rs", TEST_BODY);
    let engine = engine(&repo);

    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        for (base, target) in [("HEAD", ":index"), ("HEAD", ":worktree")] {
            assert_eq!(
                assert_snapshot_way(&engine, base, target, mode),
                [moved("src/old.rs", "src/new.rs", true)],
                "{mode:?} snapshot move {base}..{target}"
            );
        }

        assert!(
            assert_snapshot_way(&engine, ":index", ":worktree", mode).is_empty(),
            "{mode:?} index..worktree is the same path with a body-only edit"
        );
    }
}

#[test]
fn dirty_worktree_same_path_is_not_skipped_as_an_unchanged_blob() {
    let repo = TestRepo::init();
    repo.write("src/keep.rs", SIG_OLD);
    repo.commit("base");
    repo.write("src/keep.rs", SIG_NEW);

    let engine = engine(&repo);
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        for (base, target) in [("HEAD", ":worktree"), (":index", ":worktree")] {
            assert_eq!(
                assert_snapshot_way(&engine, base, target, mode),
                [modified("src/keep.rs")],
                "{mode:?} dirty worktree {base}..{target} must not be skipped"
            );
        }
    }
}

#[test]
fn unchanged_same_path_blob_is_not_read_by_any_api_or_mode() {
    let repo = TestRepo::init();
    repo.write("src/keep.rs", KEEP);
    repo.write("src/changed.rs", SIG_OLD);
    let base = repo.commit("base");
    repo.write("src/changed.rs", SIG_NEW);
    let target = repo.commit("change another file");

    let blob = repo
        .git_ok(&["rev-parse", "HEAD:src/keep.rs"])
        .trim()
        .to_owned();
    let object = repo
        .path()
        .join(".git/objects")
        .join(&blob[..2])
        .join(&blob[2..]);
    assert!(
        object.is_file(),
        "expected a loose object at {}; the no-read fixture requires it",
        object.display()
    );
    fs::remove_file(&object).expect("remove unchanged blob object");

    let engine = engine(&repo);
    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        for selection in [all(), path_selection("src/keep.rs")] {
            let events =
                assert_three_way(&engine, &base, &target, mode, &selection, DiffBase::Given);
            assert!(
                !events.iter().any(|event| {
                    event.old_path.as_deref() == Some("src/keep.rs")
                        || event.new_path.as_deref() == Some("src/keep.rs")
                }),
                "unchanged keep.rs must never be projected ({mode:?}): {events:?}"
            );
            if selection != all() {
                assert!(events.is_empty(), "unchanged selected path: {events:?}");
            }
        }
    }
}
