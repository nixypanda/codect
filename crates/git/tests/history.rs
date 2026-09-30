mod support;

use git::{GitError, GitRepository, SnapshotRepository};
use support::TestRepo;

#[test]
fn first_parent_steps_include_empty_and_merge_commits_newest_first() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub struct A;\n");
    let base = repo.commit("base");
    repo.commit_empty("empty projection\n\nbody");
    let branch_point = repo.head_id();
    repo.git_ok(&["switch", "-q", "-c", "side"]);
    repo.write("side.txt", "side\n");
    let side = repo.commit("side");
    repo.git_ok(&["switch", "-q", "main"]);
    repo.write("main.txt", "main\n");
    let main = repo.commit("main");
    repo.git_ok(&["merge", "-q", "--no-ff", "side", "-m", "merge side"]);
    let merge = repo.head_id();

    let git = GitRepository::discover(repo.path()).unwrap();
    let steps = git
        .first_parent_steps(
            &git.resolve_commit(&base).unwrap(),
            &git.resolve_commit("HEAD").unwrap(),
        )
        .unwrap();
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps
            .iter()
            .map(|step| step.subject.as_str())
            .collect::<Vec<_>>(),
        ["merge side", "main", "empty projection"]
    );
    assert_eq!(steps[0].commit_id.to_string(), merge);
    assert_eq!(steps[0].parent_id.to_string(), main);
    assert_eq!(steps[1].parent_id.to_string(), branch_point);
    assert_eq!(steps[2].parent_id.to_string(), base);
    assert!(!steps.iter().any(|step| step.commit_id.to_string() == side));

    let error = git
        .first_parent_steps(
            &git.resolve_commit(&side).unwrap(),
            &git.resolve_commit("HEAD").unwrap(),
        )
        .unwrap_err();
    assert!(matches!(error, GitError::NotFirstParentAncestor { .. }));
}

#[test]
fn equal_endpoints_are_empty_and_reverse_ancestry_is_rejected() {
    let repo = TestRepo::init();
    let base = repo.commit("base");
    let target = repo.commit_empty("target");
    let git = GitRepository::discover(repo.path()).unwrap();
    let revision = git.resolve_commit(&base).unwrap();
    assert!(
        git.first_parent_steps(&revision, &revision)
            .unwrap()
            .is_empty()
    );
    let error = git
        .first_parent_steps(&git.resolve_commit(&target).unwrap(), &revision)
        .unwrap_err();
    assert!(matches!(error, GitError::NotFirstParentAncestor { .. }));
}
