// Untracked worktree enumeration (TECHNICAL_DESIGN.md section 8).
//
// Fixtures are built with the `git` executable (see `tests/support/mod.rs`);
// the assertions exercise only `GitRepository`/`SnapshotRepository`. The
// ignore rules come from the repository, never from host configuration,
// because the support harness isolates global and system config.

mod support;

use git::repository::{GitRepository, SnapshotRepository};
use support::TestRepo;

fn untracked(repo: &TestRepo) -> Vec<String> {
    let discovered = GitRepository::discover(repo.path()).expect("discover");
    discovered
        .untracked_paths()
        .expect("enumerate untracked")
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn lists_only_supported_untracked_files_in_raw_byte_order() {
    let repo = TestRepo::init();
    repo.write("src/tracked.rs", "pub fn tracked() {}\n");
    repo.commit("initial");

    repo.write("src/zeta.rs", "pub fn zeta() {}\n");
    repo.write("src/alpha.rs", "pub fn alpha() {}\n");
    repo.write("src/alpha/nested.rs", "pub fn nested() {}\n");
    repo.write("src/notes.txt", "unsupported\n");
    repo.write("src/alpha.rs.bak", "unsupported\n");
    repo.write("src/tracked.rs", "pub fn tracked(x: u8) {}\n");

    assert_eq!(
        untracked(&repo),
        vec!["src/alpha.rs", "src/alpha/nested.rs", "src/zeta.rs",],
        "only untracked supported files appear, sorted by raw path bytes"
    );
}

#[test]
fn an_untracked_directory_is_walked_not_collapsed() {
    let repo = TestRepo::init();
    repo.write("keep.rs", "pub fn keep() {}\n");
    repo.commit("initial");

    repo.write("new/deep/one.rs", "pub fn one() {}\n");
    repo.write("new/deep/two.rs", "pub fn two() {}\n");

    assert_eq!(untracked(&repo), vec!["new/deep/one.rs", "new/deep/two.rs"]);
}

#[test]
fn ignores_ignored_paths_and_honors_negation() {
    let repo = TestRepo::init();
    repo.write(".gitignore", "target/\n*.gen.rs\n!keep.gen.rs\n");
    repo.commit("initial");

    repo.write("src/fresh.rs", "pub fn fresh() {}\n");
    repo.write("target/junk.rs", "pub fn junk() {}\n");
    repo.write("dropped.gen.rs", "pub fn dropped() {}\n");
    repo.write("keep.gen.rs", "pub fn keep() {}\n");

    assert_eq!(
        untracked(&repo),
        vec!["keep.gen.rs", "src/fresh.rs"],
        "an ignored directory is pruned and a negated pattern is honored"
    );
}

#[test]
fn nested_gitignore_applies_within_its_directory() {
    let repo = TestRepo::init();
    repo.write(".gitignore", "*.tmp\n");
    repo.commit("initial");

    // A nested ignore file only affects its own directory subtree.
    repo.write("src/.gitignore", "local.rs\n");
    repo.write("src/local.rs", "pub fn local() {}\n");
    repo.write("src/other.rs", "pub fn other() {}\n");
    repo.write("other/local.rs", "pub fn elsewhere() {}\n");

    assert_eq!(untracked(&repo), vec!["other/local.rs", "src/other.rs"]);
}

#[test]
fn info_exclude_is_honored() {
    let repo = TestRepo::init();
    repo.write(".git/info/exclude", "excluded.rs\n");
    repo.write("tracked.rs", "pub fn tracked() {}\n");
    repo.commit("initial");

    repo.write("excluded.rs", "pub fn excluded() {}\n");
    repo.write("kept.rs", "pub fn kept() {}\n");

    assert_eq!(untracked(&repo), vec!["kept.rs"]);
}

#[test]
fn symlinks_and_directories_are_never_untracked_files() {
    let repo = TestRepo::init();
    repo.write("real.rs", "pub fn real() {}\n");
    repo.commit("initial");

    repo.write("dir/inside.rs", "pub fn inside() {}\n");
    std::os::unix::fs::symlink("real.rs", repo.path().join("link.rs")).expect("symlink");

    // The symlink is skipped; the untracked directory is walked.
    assert_eq!(untracked(&repo), vec!["dir/inside.rs"]);
}

#[test]
fn a_bare_repository_has_no_untracked_files() {
    let repo = TestRepo::init_bare();
    let discovered = GitRepository::discover(repo.path()).expect("discover");
    assert_eq!(discovered.untracked_paths().expect("enumerate"), Vec::new());
}
