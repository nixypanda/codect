//! Repository discovery integration tests (TECHNICAL_DESIGN.md section 8.1).

mod support;

use git::GitError;
use git::repository::{GitRepository, SnapshotRepository};
use support::TestRepo;

#[test]
fn discovers_a_normal_repository_from_a_subdirectory() {
    let repo = TestRepo::init();
    repo.write("src/nested/lib.rs", "pub fn nested() {}\n");
    repo.commit("initial");

    let subdirectory = repo.child("src/nested");
    let discovered = GitRepository::discover(&subdirectory).expect("discover from a subdirectory");

    assert!(discovered.git_dir().ends_with(".git"));
    let revision = discovered.resolve_commit("HEAD").expect("resolve HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.to_string(), "src/nested/lib.rs");
}

#[test]
fn discovers_a_bare_repository() {
    let source = TestRepo::init();
    source.write("src/lib.rs", "pub fn bare() {}\n");
    source.commit("initial");

    let bare = source.child("bare.git");
    source.git_ok(&[
        "clone",
        "-q",
        "--bare",
        source.path().to_str().expect("utf-8 path"),
        bare.to_str().expect("utf-8 path"),
    ]);

    let discovered = GitRepository::discover(&bare).expect("discover a bare repository");
    assert!(discovered.work_dir().is_none());

    let revision = discovered.resolve_commit("HEAD").expect("resolve HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.to_string(), "src/lib.rs");
}

#[test]
fn discovers_a_linked_worktree() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn main_branch() {}\n");
    repo.commit("initial");

    let worktree = repo.child("wt");
    repo.git_ok(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "wt-branch",
        worktree.to_str().expect("utf-8 path"),
    ]);

    let discovered = GitRepository::discover(&worktree).expect("discover a linked worktree");
    assert!(discovered.work_dir().is_some());

    let revision = discovered
        .resolve_commit("wt-branch")
        .expect("resolve worktree branch");
    let entries = discovered.source_entries(&revision).expect("enumerate");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.to_string(), "src/lib.rs");
}

#[test]
fn reports_a_clear_error_when_no_repository_is_found() {
    let directory = TestRepo::new();
    let nested = directory.child("not/a/repository");
    std::fs::create_dir_all(&nested).expect("create nested directory");

    let error = GitRepository::discover(&nested).expect_err("no repository should be found");
    assert!(matches!(error, GitError::RepositoryNotFound { .. }));
    assert!(
        error.to_string().contains("no Git repository found"),
        "unexpected message: {error}"
    );
}
