//! Engine-level integration tests over real temporary Git repositories.
//!
//! These exercise the shared pipeline directly: ordering, empty projections,
//! focused-diff filtering, selection validation, and the per-operation caches.
//! The CLI and terminal frontends build on exactly these results.

mod support;

use ownai_core::{ProjectionMode, RepoPath};
use ownai_engine::{Engine, EngineError, FileDiff, Selection, SelectionGroup};
use support::TestRepo;

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

const RUST_TYPE_VARIANT: &str = "\
pub struct User {
    pub id: u64,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

const EMPTY_RUST: &str = "// no declarations here\n";

fn engine(repo: &TestRepo) -> Engine {
    Engine::discover(repo.path()).expect("discover repository")
}

fn path_selection(paths: &[&str]) -> Selection {
    let groups = paths
        .iter()
        .map(|raw| SelectionGroup::Path {
            label: (*raw).to_owned(),
            path: RepoPath::new(*raw).expect("valid path"),
        })
        .collect();
    Selection::new(groups).expect("valid selection")
}

fn area_selection(name: &str, paths: &[&str]) -> Selection {
    let paths = paths
        .iter()
        .map(|raw| RepoPath::new(*raw).expect("valid path"))
        .collect();
    Selection::new(vec![SelectionGroup::Area {
        name: name.to_owned(),
        paths,
    }])
    .expect("valid selection")
}

fn paths_of(diffs: &[FileDiff]) -> Vec<String> {
    diffs.iter().map(|diff| diff.path.to_string()).collect()
}

#[test]
fn show_returns_supported_files_in_raw_path_byte_order() {
    let repo = TestRepo::init();
    repo.write("b.rs", RUST_BASE);
    repo.write("a.rs", RUST_BASE);
    repo.write("README.md", "not projected");
    repo.commit("base");

    let files = engine(&repo)
        .show("HEAD", ProjectionMode::Types, &Selection::all())
        .expect("show");

    let names: Vec<String> = files.iter().map(|file| file.path().to_string()).collect();
    assert_eq!(names, vec!["a.rs", "b.rs"]);
    assert!(files.iter().all(|file| !file.canonical_text().is_empty()));
}

#[test]
fn show_scopes_to_the_selected_paths() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.write("b.rs", RUST_BASE);
    repo.commit("base");

    let files = engine(&repo)
        .show("HEAD", ProjectionMode::Types, &path_selection(&["a.rs"]))
        .expect("show");

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path().to_string(), "a.rs");
}

#[test]
fn show_includes_a_file_with_an_empty_projection() {
    let repo = TestRepo::init();
    repo.write("empty.rs", EMPTY_RUST);
    repo.write("full.rs", RUST_BASE);
    repo.commit("base");

    let files = engine(&repo)
        .show("HEAD", ProjectionMode::Types, &Selection::all())
        .expect("show");

    let names: Vec<String> = files.iter().map(|file| file.path().to_string()).collect();
    assert_eq!(names, vec!["empty.rs", "full.rs"]);
    assert_eq!(files[0].canonical_text(), "");
}

#[test]
fn show_rejects_a_selection_that_names_nothing() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.commit("base");

    let error = engine(&repo)
        .show(
            "HEAD",
            ProjectionMode::Types,
            &path_selection(&["missing.rs"]),
        )
        .expect_err("unsatisfied selection");

    match error {
        EngineError::UnsatisfiedSelection { revisions, missing } => {
            assert_eq!(revisions, "`HEAD`");
            assert_eq!(missing.len(), 1);
            assert_eq!(missing[0].kind_label(), "path");
            assert_eq!(missing[0].label(), "missing.rs");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn an_area_is_satisfied_by_any_one_of_its_paths() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.write(".ownai.toml", "[areas]\nfrontend = [\"a.rs\", \"gone\"]\n");
    repo.commit("base");

    let engine = engine(&repo);
    let areas = engine.load_areas().expect("load areas");
    assert_eq!(areas.names().collect::<Vec<_>>(), vec!["frontend"]);

    let files = engine
        .show(
            "HEAD",
            ProjectionMode::Types,
            &area_selection("frontend", &["a.rs", "gone"]),
        )
        .expect("show");
    assert_eq!(files.len(), 1);
}

#[test]
fn an_area_absent_from_the_revision_is_unsatisfied() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.commit("base");

    let error = engine(&repo)
        .show(
            "HEAD",
            ProjectionMode::Types,
            &area_selection("frontend", &["apps/web", "packages/ui"]),
        )
        .expect_err("unsatisfied selection");

    match error {
        EngineError::UnsatisfiedSelection { missing, .. } => {
            assert_eq!(missing[0].kind_label(), "area");
            assert_eq!(missing[0].label(), "frontend");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn a_malformed_config_is_reported_as_a_config_error() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.write(".ownai.toml", "this is not toml\n");
    repo.commit("base");

    let error = engine(&repo).load_areas().expect_err("config error");
    assert!(matches!(error, EngineError::Config(_)), "got {error:?}");
}

#[test]
fn diff_keeps_only_changed_projections() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_TYPE_VARIANT);
    repo.commit("type change");

    let engine = engine(&repo);
    let types = engine
        .diff("HEAD~1", "HEAD", ProjectionMode::Types, &Selection::all())
        .expect("diff types");
    assert_eq!(paths_of(&types), vec!["src/lib.rs"]);
    assert!(types[0].old.is_some() && types[0].new.is_some());
}

#[test]
fn diff_body_only_changes_are_invisible() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_BODY_VARIANT);
    repo.commit("body change");

    let engine = engine(&repo);
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let diffs = engine
            .diff("HEAD~1", "HEAD", mode, &Selection::all())
            .expect("diff");
        assert!(diffs.is_empty(), "expected no diff in {mode:?}");
    }
}

#[test]
fn diff_marks_added_and_deleted_files() {
    let repo = TestRepo::init();
    repo.write("gone.rs", RUST_BASE);
    repo.commit("base");
    repo.remove("gone.rs");
    repo.write("fresh.rs", RUST_BASE);
    repo.commit("swap");

    let diffs = engine(&repo)
        .diff("HEAD~1", "HEAD", ProjectionMode::Types, &Selection::all())
        .expect("diff");

    assert_eq!(paths_of(&diffs), vec!["fresh.rs", "gone.rs"]);
    assert!(diffs[0].old.is_none() && diffs[0].new.is_some());
    assert!(diffs[1].old.is_some() && diffs[1].new.is_none());
}

#[test]
fn diff_treats_an_empty_projection_as_absent_content() {
    let repo = TestRepo::init();
    repo.write("empty.rs", EMPTY_RUST);
    repo.commit("base");
    repo.remove("empty.rs");
    repo.commit("delete");

    let diffs = engine(&repo)
        .diff("HEAD~1", "HEAD", ProjectionMode::Types, &Selection::all())
        .expect("diff");

    assert!(
        diffs.is_empty(),
        "an empty projection equals an absent side"
    );
}

#[test]
fn diff_rejects_a_path_absent_from_both_revisions() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.commit("base");
    repo.write("a.rs", RUST_TYPE_VARIANT);
    repo.commit("change");

    let error = engine(&repo)
        .diff(
            "HEAD~1",
            "HEAD",
            ProjectionMode::Types,
            &path_selection(&["missing.rs"]),
        )
        .expect_err("unsatisfied selection");

    match error {
        EngineError::UnsatisfiedSelection { revisions, .. } => {
            assert_eq!(revisions, "`HEAD~1` or `HEAD`");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn diff_accepts_a_path_deleted_by_the_target() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.commit("base");
    repo.remove("a.rs");
    repo.commit("delete");

    let diffs = engine(&repo)
        .diff(
            "HEAD~1",
            "HEAD",
            ProjectionMode::Types,
            &path_selection(&["a.rs"]),
        )
        .expect("diff");

    assert_eq!(paths_of(&diffs), vec!["a.rs"]);
    assert!(diffs[0].old.is_some() && diffs[0].new.is_none());
}

#[test]
fn the_projection_cache_keys_on_blob_not_path() {
    let repo = TestRepo::init();
    repo.write("one.rs", RUST_BASE);
    repo.write("two.rs", RUST_BASE);
    repo.commit("base");

    let files = engine(&repo)
        .show("HEAD", ProjectionMode::Types, &Selection::all())
        .expect("show");

    assert_eq!(
        files
            .iter()
            .map(|file| file.path().to_string())
            .collect::<Vec<_>>(),
        vec!["one.rs", "two.rs"]
    );
    // Identical blobs share cached items but each file keeps its own path.
    assert_eq!(files[0].canonical_text(), files[1].canonical_text());
}

#[test]
fn show_and_diff_work_in_a_bare_repository() {
    let normal = TestRepo::init();
    normal.write("src/lib.rs", RUST_BASE);
    normal.commit("base");
    normal.write("src/lib.rs", RUST_TYPE_VARIANT);
    normal.commit("change");

    let bare = normal.clone_bare();
    let engine = Engine::discover(bare.path()).expect("discover bare repository");

    let files = engine
        .show("HEAD", ProjectionMode::Types, &Selection::all())
        .expect("show");
    assert_eq!(files.len(), 1);

    let diffs = engine
        .diff("HEAD~1", "HEAD", ProjectionMode::Types, &Selection::all())
        .expect("diff");
    assert_eq!(paths_of(&diffs), vec!["src/lib.rs"]);
}

#[test]
fn root_is_the_worktree_for_a_normal_repository() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.commit("base");

    // `/private/var` and `/var` aliases make a byte comparison unreliable, so
    // the root is checked by its trailing component.
    let root = engine(&repo).root().to_path_buf();
    assert!(root.ends_with(repo.path().file_name().unwrap()));
}

#[test]
fn load_review_config_defaults_without_a_file_and_reads_a_configured_taxonomy() {
    let repo = TestRepo::init();
    repo.write("a.rs", RUST_BASE);
    repo.commit("base");

    let engine = engine(&repo);
    let defaults = engine.load_review_config().expect("defaults");
    assert_eq!(defaults.choice_confidence_threshold, 0.55);
    assert!(defaults.concerns.is_none());

    repo.write(
        ".ownai.toml",
        concat!(
            "[lenses.review]\n",
            "choice_confidence_threshold = 0.9\n",
            "\n",
            "[lenses.review.concerns]\n",
            "engine = \"Engine behavior\"\n",
            "other = \"No listed concern\"\n",
        ),
    );

    let configured = engine.load_review_config().expect("configured");
    assert_eq!(configured.choice_confidence_threshold, 0.9);
    let concerns = configured.concerns.expect("configured concerns");
    let keys: Vec<&str> = concerns
        .options()
        .iter()
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(keys, vec!["engine", "other"]);
}
