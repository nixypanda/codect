//! Engine-level integration tests over real temporary Git repositories.
//!
//! These exercise the shared pipeline directly: empty projections, selection
//! validation and its error shapes, stable keys, and repository discovery. The
//! ordering and focused-diff filtering rules are covered end to end by the CLI
//! suite; the CLI and terminal frontends build on exactly these results.

mod support;

use ownai_core::{Area, ProjectionMode, RepoPath, Selection, SelectionGroup};
use ownai_engine::{Engine, EngineError};
use support::TestRepo;

#[test]
fn first_parent_history_exposes_steps_without_projection() {
    let repo = TestRepo::init();
    let base = repo.commit("base");
    let target = repo.commit("metadata-only step");
    let steps = engine(&repo).first_parent_steps(&base, "HEAD").unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].parent_id.to_string(), base);
    assert_eq!(steps[0].commit_id.to_string(), target);
    assert_eq!(steps[0].subject, "metadata-only step");
}

const RUST_BASE: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
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

const RUST_MODULE: &str = "\
pub mod outer {
    pub fn f() {}
}
";

const PYTHON_CLASS: &str = "\
class Widget:
    def render(self):
        return 1
";

fn engine(repo: &TestRepo) -> Engine {
    Engine::discover(repo.path()).expect("discover repository")
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
fn identical_blobs_project_the_same_text_at_each_path() {
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
    // Identical blobs project identical text, and each file keeps its own path.
    assert_eq!(files[0].canonical_text(), files[1].canonical_text());
}

#[test]
fn identical_blobs_at_different_paths_get_path_correct_stable_keys() {
    let repo = TestRepo::init();
    repo.write("a/m.rs", RUST_MODULE);
    repo.write("b/m.rs", RUST_MODULE);
    repo.write("a/m.py", PYTHON_CLASS);
    repo.write("b/m.py", PYTHON_CLASS);
    repo.commit("base");

    let files = engine(&repo)
        .show_outlines("HEAD", ProjectionMode::Types, &Selection::all())
        .expect("show outlines");

    let keys_of = |raw: &str| -> Vec<String> {
        let file = files
            .iter()
            .find(|file| file.path().to_string() == raw)
            .unwrap_or_else(|| panic!("no outline for {raw}"));
        file.outline
            .iter()
            .map(|item| item.item.stable_key.clone())
            .collect()
    };

    // Rust `mod` stable keys embed the repository path, so the second file must
    // not reuse the first file's cached keys even though the blob is identical.
    assert_eq!(
        keys_of("a/m.rs"),
        vec!["a/m.rs::mod::outer", "a/m.rs::mod::outer::fn::f"]
    );
    assert_eq!(
        keys_of("b/m.rs"),
        vec!["b/m.rs::mod::outer", "b/m.rs::mod::outer::fn::f"]
    );

    // Python `class` keys are path-namespaced too.
    assert_eq!(
        keys_of("a/m.py"),
        vec![
            "a/m.py::class::Widget",
            "a/m.py::class::Widget::method::render"
        ]
    );
    assert_eq!(
        keys_of("b/m.py"),
        vec![
            "b/m.py::class::Widget",
            "b/m.py::class::Widget::method::render"
        ]
    );
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
