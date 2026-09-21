//! Snapshot invariants and the Phase 2 acceptance gate
//! (TECHNICAL_DESIGN.md sections 8 and 19).
//!
//! Every test here builds fixtures with the `git` executable (see
//! `tests/support/mod.rs`) and then uses only `GitRepository` and
//! `SnapshotRepository` to enumerate and read committed files.

mod support;

use std::collections::BTreeMap;

use ownai_core::RepoPath;
use ownai_git::repository::{GitRepository, SnapshotRepository};
use support::TestRepo;

#[test]
fn enumerates_and_reads_elm_and_rust_from_two_commits() {
    let repo = TestRepo::init();

    repo.write(
        "src/Model.elm",
        "module Model exposing (..)\n\ntype alias Model = Int\n",
    );
    repo.write("src/lib.rs", "pub fn first() -> u8 { 1 }\n");
    repo.write("README.md", "ignored\n");
    let first = repo.commit("first");

    repo.write(
        "src/Model.elm",
        "module Model exposing (..)\n\ntype alias Model = String\n",
    );
    repo.write("src/main.rs", "fn main() {}\n");
    repo.write("src/lib.rs", "pub fn first() -> u8 { 1 }\n");
    let second = repo.commit("second");

    let discovered = GitRepository::discover(repo.path()).expect("discover");

    let first_revision = discovered.resolve_commit(&first).expect("first commit");
    let second_revision = discovered.resolve_commit(&second).expect("second commit");

    let first_entries = discovered
        .source_entries(&first_revision)
        .expect("enumerate first");
    let second_entries = discovered
        .source_entries(&second_revision)
        .expect("enumerate second");

    assert_eq!(
        first_entries
            .iter()
            .map(|entry| entry.path.to_string())
            .collect::<Vec<_>>(),
        vec!["src/Model.elm", "src/lib.rs"]
    );
    assert_eq!(
        second_entries
            .iter()
            .map(|entry| entry.path.to_string())
            .collect::<Vec<_>>(),
        vec!["src/Model.elm", "src/lib.rs", "src/main.rs"]
    );

    let expected = BTreeMap::from([
        (
            "src/Model.elm",
            "module Model exposing (..)\n\ntype alias Model = String\n",
        ),
        ("src/lib.rs", "pub fn first() -> u8 { 1 }\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]);

    for entry in &second_entries {
        let path = entry.path.to_string();
        let language = entry.path.language().expect("supported path");
        let bytes = discovered.read_blob(&entry.blob_id).expect("read blob");
        let source = ownai_core::decode_source(&entry.path, language, &bytes).expect("utf-8");
        assert_eq!(source, expected[path.as_str()], "contents of {path}");
    }

    let first_ids: BTreeMap<String, String> = first_entries
        .iter()
        .map(|entry| (entry.path.to_string(), entry.blob_id.to_string()))
        .collect();
    let second_ids: BTreeMap<String, String> = second_entries
        .iter()
        .map(|entry| (entry.path.to_string(), entry.blob_id.to_string()))
        .collect();
    assert_eq!(first_ids["src/lib.rs"], second_ids["src/lib.rs"]);
    assert_ne!(first_ids["src/Model.elm"], second_ids["src/Model.elm"]);
}

#[test]
fn read_blob_returns_exact_bytes() {
    let repo = TestRepo::init();

    // No trailing newline, CRLF line endings, a NUL byte, and multibyte UTF-8.
    let exact: &[u8] = b"\xEF\xB7\xBFmodule Exact exposing (..)\r\nvalue = 1\r\n\0";
    repo.write_bytes("src/Exact.elm", exact);
    repo.write_bytes("src/exact.rs", b"pub const N: u8 = 0;\n");
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");

    let by_path: BTreeMap<&RepoPath, _> =
        entries.iter().map(|entry| (&entry.path, entry)).collect();
    let elm = by_path
        .get(&RepoPath::new("src/Exact.elm").expect("path"))
        .expect("Elm entry");
    let bytes = discovered.read_blob(&elm.blob_id).expect("read blob");
    assert_eq!(bytes, exact);
}

#[test]
fn source_entries_are_sorted_by_raw_path_bytes() {
    let repo = TestRepo::init();
    for path in [
        "src/zeta.rs",
        "src/alpha.rs",
        "src/alpha/nested.rs",
        "src/alpha.rs.bak", // unsupported, ignored
        "a.rs",
        "b.elm",
    ] {
        repo.write(path, "// fixture\n");
    }
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");

    for window in entries.windows(2) {
        assert!(
            window[0].path.as_bytes() < window[1].path.as_bytes(),
            "paths must be strictly increasing by raw bytes: {} then {}",
            window[0].path,
            window[1].path
        );
    }
}
