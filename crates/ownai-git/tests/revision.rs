//! Revision resolution and peeling integration tests
//! (TECHNICAL_DESIGN.md section 8.2).

mod support;

use ownai_git::GitError;
use ownai_git::repository::{GitRepository, ObjectId, SnapshotRepository};
use support::{TestRepo, find_ambiguous_prefix};

/// Decodes a hexadecimal object id produced by the Git executable.
fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("valid hex"))
        .collect()
}

fn object_id(hex: &str) -> ObjectId {
    ObjectId {
        kind: ownai_git::repository::HashKind::Sha1,
        bytes: hex_to_bytes(hex),
    }
}

#[test]
fn resolves_head_branch_tag_full_and_abbreviated_ids() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    let head = repo.commit("initial");
    repo.git_ok(&["tag", "lightweight"]);
    repo.git_ok(&["branch", "feature"]);
    repo.tag_annotated("annotated", "annotated release");

    let discovered = GitRepository::discover(repo.path()).expect("discover");

    let expected = object_id(&head);
    for spec in ["HEAD", "main", "feature", "lightweight", "annotated"] {
        let revision = discovered.resolve_commit(spec).expect(spec);
        assert_eq!(revision.object_id, expected, "spec `{spec}`");
    }

    let abbreviated = &head[..8];
    let revision = discovered
        .resolve_commit(abbreviated)
        .expect("abbreviated id");
    assert_eq!(revision.object_id, expected);
}

#[test]
fn resolves_parent_and_ancestor_suffixes() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    let first = repo.commit("first");
    repo.write("src/lib.rs", "pub fn two() {}\n");
    repo.commit("second");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let first = object_id(&first);

    assert_eq!(
        discovered
            .resolve_commit("HEAD~1")
            .expect("HEAD~1")
            .object_id,
        first
    );
    assert_eq!(
        discovered.resolve_commit("HEAD^").expect("HEAD^").object_id,
        first
    );

    // The full id of HEAD and the symbolic spec must agree.
    let head = discovered.resolve_commit("HEAD").expect("HEAD");
    let resolved = discovered
        .resolve_commit(&head.object_id.to_string())
        .expect("full id");
    assert_eq!(resolved, head);
}

#[test]
fn peels_an_annotated_tag_to_its_commit() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    let head = repo.commit("initial");
    repo.tag_annotated("v1.0.0", "release 1.0.0");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("v1.0.0").expect("annotated tag");
    assert_eq!(revision.object_id, object_id(&head));
}

#[test]
fn rejects_ranges_passed_as_one_argument() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    repo.commit("first");
    repo.write("src/lib.rs", "pub fn two() {}\n");
    repo.commit("second");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    for spec in ["HEAD~1..HEAD", "HEAD~1...HEAD"] {
        let error = discovered
            .resolve_commit(spec)
            .expect_err("range must be rejected");
        assert!(
            matches!(error, GitError::RevisionRange { .. }),
            "spec `{spec}` produced {error:?}"
        );
    }
}

#[test]
fn rejects_a_tree_that_cannot_peel_to_a_commit() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    repo.commit("initial");
    let tree = repo.rev_parse("HEAD^{tree}");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let error = discovered
        .resolve_commit(&tree)
        .expect_err("a tree is not a commit");
    assert!(
        matches!(error, GitError::NotPeelable { .. }),
        "unexpected error: {error:?}"
    );
}

#[test]
fn rejects_a_blob_that_cannot_peel_to_a_commit() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    repo.commit("initial");
    let blob = repo.rev_parse("HEAD:src/lib.rs");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let error = discovered
        .resolve_commit(&blob)
        .expect_err("a blob is not a commit");
    assert!(
        matches!(error, GitError::NotPeelable { .. }),
        "unexpected error: {error:?}"
    );
}

#[test]
fn reports_a_revision_that_does_not_exist() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn one() {}\n");
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    for spec in ["does-not-exist", "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"] {
        let error = discovered
            .resolve_commit(spec)
            .expect_err("unknown revision");
        assert!(
            matches!(error, GitError::RevisionNotFound { .. }),
            "spec `{spec}` produced {error:?}"
        );
    }
}

#[test]
fn reports_an_ambiguous_abbreviated_revision() {
    // Build enough objects that a four-hex-digit prefix is shared by two of
    // them. `core.disambiguate=none` keeps host configuration from changing
    // how the prefix is resolved.
    let repo = TestRepo::init();
    for index in 0..2048 {
        repo.write(
            &format!("blob/{index}.dat"),
            &format!("fixture object {index}\n"),
        );
    }
    repo.commit("many objects");
    repo.git_ok(&["config", "core.disambiguate", "none"]);

    let prefix =
        find_ambiguous_prefix(repo.path()).expect("expected two objects sharing a four-hex prefix");
    assert_eq!(prefix.len(), 4);

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let error = discovered
        .resolve_commit(&prefix)
        .expect_err("abbreviated revision must be ambiguous");
    assert!(
        matches!(error, GitError::AmbiguousRevision { .. }),
        "unexpected error: {error:?}"
    );
}

#[test]
fn reports_an_unborn_head() {
    let repo = TestRepo::init();

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let error = discovered.resolve_commit("HEAD").expect_err("unborn HEAD");
    assert!(
        matches!(error, GitError::RevisionNotFound { .. }),
        "unexpected error: {error:?}"
    );
}

#[test]
fn resolves_identically_from_different_discovery_starting_points() {
    // The same repository opened from two starting points resolves identically.
    let repo = TestRepo::init();
    repo.write("src/nested/lib.rs", "pub fn one() {}\n");
    let head = repo.commit("initial");

    let from_root = GitRepository::discover(repo.path()).expect("discover root");
    let from_nested = GitRepository::discover(repo.child("src/nested")).expect("discover nested");

    assert_eq!(
        from_root.resolve_commit("HEAD").expect("root"),
        from_nested.resolve_commit("HEAD").expect("nested")
    );
    assert_eq!(
        from_root.resolve_commit("HEAD").expect("root").object_id,
        object_id(&head)
    );
}
