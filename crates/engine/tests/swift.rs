// Swift acceptance at the engine layer.
//
// These tests exercise the shared engine pipeline with Swift source: the frozen
// contract's canonical shapes, the mode-independent outline, snapshots, path
// scoping, untracked worktree files, and same-language exact/fuzzy moves. They
// use real temporary Git repositories; the engine under test never invokes Git.
//
// Expected text is taken from the frozen contract in
// `docs/plans/swift-language-contract.md`, not from an implementation snapshot.

mod support;

use std::collections::BTreeSet;

use base::{
    Area, FileDiff, FileOutline, FileOutlineDiff, ProjectionMode, RepoPath, Selection,
    SelectionGroup,
};
use engine::{DiffBase, Engine, project_source};
use support::TestRepo;

// §8.1 of the contract.
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

// §11 of the contract. Independent top-level declarations are separated by one
// blank line; nested members keep a single newline.
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

fn project(path: &str, source: &str, mode: ProjectionMode) -> FileOutline {
    project_source(
        &RepoPath::new(path).expect("valid path"),
        source.as_bytes(),
        mode,
    )
    .unwrap_or_else(|error| panic!("{mode:?} projection of {path}: {error:?}"))
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

// A Swift file long enough that a one-line edit is still a high-similarity
// fuzzy move candidate. Source bytes, not projected text, decide pairing.
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
fn swift_types_and_signatures_match_the_frozen_golden() {
    let types = project("src/Point.swift", POINT, ProjectionMode::Types);
    let signatures = project("src/Point.swift", POINT, ProjectionMode::Signatures);

    assert_eq!(types.projection.canonical_text(), POINT_TYPES);
    assert_eq!(signatures.projection.canonical_text(), POINT_SIGNATURES);
}

#[test]
fn swift_tests_mode_keeps_only_tests_and_their_containers() {
    // Add a non-test declaration so the outline-fallback assertion is not
    // vacuous: it must be located by the outline but dropped from Tests.
    let source = format!("{SWIFT_TESTS}\nfunc helper() {{}}\n");
    let tests = project("src/Tests.swift", &source, ProjectionMode::Tests);
    assert_eq!(tests.projection.canonical_text(), SWIFT_TESTS_GOLDEN);

    let signatures = project("src/Tests.swift", &source, ProjectionMode::Signatures);
    let signature_keys: BTreeSet<&str> = signatures
        .projection
        .items()
        .iter()
        .map(|item| item.stable_key.as_str())
        .collect();

    // Tests mode declares no types and is a strict filter of Signatures.
    for item in tests.projection.items() {
        assert!(
            signature_keys.contains(item.stable_key.as_str()),
            "Tests item `{}` is missing from Signatures",
            item.stable_key
        );
    }
    assert_eq!(
        tests
            .outline
            .iter()
            .map(|item| item.item.stable_key.clone())
            .collect::<Vec<_>>(),
        signatures
            .projection
            .items()
            .iter()
            .map(|item| item.stable_key.clone())
            .collect::<Vec<_>>(),
        "the Tests-mode outline must enumerate the Signatures superset"
    );
    assert!(
        tests
            .outline
            .iter()
            .any(|item| !tests.retained_in_mode(item)),
        "a non-test declaration must still be located by the outline"
    );
}

#[test]
fn swift_show_and_outlines_are_mode_independent_over_a_mixed_repository() {
    let repo = TestRepo::init();
    repo.write("src/Point.swift", POINT);
    repo.write("src/lib.rs", "pub struct Keep;\n");
    repo.commit("base");

    let engine = engine(&repo);
    let types = engine
        .show(
            "HEAD",
            ProjectionMode::Types,
            &path_selection("src/Point.swift"),
        )
        .expect("show types");
    let signatures = engine
        .show(
            "HEAD",
            ProjectionMode::Signatures,
            &path_selection("src/Point.swift"),
        )
        .expect("show signatures");

    assert_eq!(types.len(), 1, "only the selected Swift file is shown");
    assert_eq!(types[0].canonical_text(), POINT_TYPES);
    assert_eq!(signatures[0].canonical_text(), POINT_SIGNATURES);

    let types_outline = engine
        .show_outlines(
            "HEAD",
            ProjectionMode::Types,
            &path_selection("src/Point.swift"),
        )
        .expect("types outlines");
    let signatures_outline = engine
        .show_outlines(
            "HEAD",
            ProjectionMode::Signatures,
            &path_selection("src/Point.swift"),
        )
        .expect("signatures outlines");
    assert_eq!(
        types_outline[0]
            .outline
            .iter()
            .map(|item| item.item.stable_key.clone())
            .collect::<Vec<_>>(),
        signatures_outline[0]
            .outline
            .iter()
            .map(|item| item.item.stable_key.clone())
            .collect::<Vec<_>>(),
        "the Types outline must enumerate the Signatures superset"
    );
}

#[test]
fn swift_exact_move_is_one_event_in_all_three_diff_apis() {
    let repo = TestRepo::init();
    repo.write("src/old.swift", POINT);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.swift", "src/new.swift"]);
    let target = repo.commit("move");

    let engine = engine(&repo);
    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        // POINT carries no test, so Tests drops the move (two empty projections).
        let expected: Vec<(String, String)> = if mode == ProjectionMode::Tests {
            vec![]
        } else {
            vec![("src/old.swift".to_owned(), "src/new.swift".to_owned())]
        };

        let projections = engine
            .diff(&base, &target, mode, &all(), DiffBase::Given)
            .expect("projection diff");
        assert_eq!(moved_paths(&projections), expected, "{mode:?} diff");

        let outlines = engine
            .diff_outlines(&base, &target, mode, &all(), DiffBase::Given)
            .expect("outline diff");
        assert_eq!(
            outline_moved_paths(&outlines.files),
            expected,
            "{mode:?} outline diff"
        );

        let snapshots = engine
            .diff_snapshot_outlines(&base, &target, mode, &all(), DiffBase::Given)
            .expect("snapshot diff");
        assert_eq!(
            outline_moved_paths(&snapshots.files),
            expected,
            "{mode:?} snapshot diff"
        );
    }
}

#[test]
fn swift_body_only_move_stays_equal_with_no_focused_change() {
    let before = "\
struct Value {
    func read() -> Int { 1 }
}
";
    let after = "\
struct Value {
    func read() -> Int { 2 }
}
";
    let repo = TestRepo::init();
    repo.write("src/old.swift", before);
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.swift", "src/new.swift"]);
    repo.write("src/new.swift", after);
    let target = repo.commit("body edit inside move");

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
    let FileDiff::Moved { old, new } = &diffs[0] else {
        panic!("expected a move, got {:?}", diffs[0]);
    };
    assert_eq!(old.canonical_text(), new.canonical_text());
    assert!(
        old.canonical_text().contains("func read() -> Int { ... }"),
        "{}",
        old.canonical_text()
    );
}

#[test]
fn swift_fuzzy_move_pairs_edited_sources_of_the_same_language() {
    let old = big_source(false);
    let new = big_source(true);
    let repo = TestRepo::init();
    repo.write("src/old.swift", &old);
    let base = repo.commit("base");
    repo.remove("src/old.swift");
    repo.write("src/new.swift", &new);
    let target = repo.commit("edited move");

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
        moved_paths(&diffs),
        [("src/old.swift".to_owned(), "src/new.swift".to_owned())],
        "a one-line edit must still pair as a fuzzy move"
    );
    match &diffs[0] {
        FileDiff::Moved { old, new } => {
            assert!(!old.canonical_text().is_empty());
            assert_ne!(old.canonical_text(), new.canonical_text());
        }
        other => panic!("expected a move, got {other:?}"),
    }
}

#[test]
fn swift_does_not_pair_with_a_different_language() {
    let repo = TestRepo::init();
    // A long Swift name keeps the two sources otherwise similar, so language
    // identity is the only reason they cannot pair.
    let name = "A".repeat(500);
    repo.write("old.swift", &format!("struct {name} {{}}\n"));
    let base = repo.commit("base");
    repo.remove("old.swift");
    repo.write("new.rs", &format!("pub struct {name};\n"));
    let target = repo.commit("cross language");

    let diffs = engine(&repo)
        .diff(
            &base,
            &target,
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("diff");
    assert_eq!(statuses(&diffs), ["added", "deleted"]);
    assert!(moved_paths(&diffs).is_empty());
}

#[test]
fn swift_snapshot_moves_cover_index_worktree_and_empty() {
    let repo = TestRepo::init();
    repo.write("src/old.swift", POINT);
    repo.commit("base");
    repo.git_ok(&["mv", "src/old.swift", "src/new.swift"]);

    let engine = engine(&repo);
    for (base, target) in [("HEAD", ":index"), ("HEAD", ":worktree")] {
        let files = engine
            .diff_snapshot_outlines(base, target, ProjectionMode::Types, &all(), DiffBase::Given)
            .expect("snapshot diff")
            .files;
        assert_eq!(
            outline_moved_paths(&files),
            [("src/old.swift".to_owned(), "src/new.swift".to_owned())],
            "{base}..{target}"
        );
    }

    let from_empty = engine
        .diff_snapshot_outlines(
            ":empty",
            "HEAD",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("empty to HEAD");
    assert_eq!(
        from_empty.files.len(),
        1,
        "the old Swift path is added from :empty"
    );

    let untracked = TestRepo::init();
    untracked.write("tracked.swift", POINT);
    untracked.commit("base");
    untracked.write("fresh.swift", POINT);
    let untracked_engine =
        Engine::discover(untracked.path()).expect("discover untracked repository");
    let files = untracked_engine
        .diff_snapshot_outlines(
            "HEAD",
            ":worktree",
            ProjectionMode::Types,
            &all(),
            DiffBase::Given,
        )
        .expect("worktree diff")
        .files;
    let paths: Vec<String> = files.iter().map(|file| file.path().to_string()).collect();
    assert_eq!(
        paths,
        ["fresh.swift"],
        "untracked Swift files are worktree members"
    );
}

#[test]
fn swift_selection_of_a_path_and_of_an_area_narrows_the_diff() {
    let repo = TestRepo::init();
    repo.write("src/old.swift", POINT);
    repo.write("other/keep.swift", POINT);
    repo.write(".codect.toml", "[areas]\nsources = [\"src\"]\n");
    let base = repo.commit("base");
    repo.git_ok(&["mv", "src/old.swift", "src/new.swift"]);
    let target = repo.commit("move");

    let area =
        Area::new("sources", [RepoPath::new("src").expect("valid path")]).expect("valid area");
    let area_selection = Selection::new(vec![SelectionGroup::Area(area)]);
    for selection in [path_selection("src/old.swift"), area_selection] {
        let diffs = engine(&repo)
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
            [("src/old.swift".to_owned(), "src/new.swift".to_owned())],
            "either path endpoint or the area selects the whole move"
        );
    }
}
