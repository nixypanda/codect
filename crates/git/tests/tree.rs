//! Commit tree traversal integration tests
//! (TECHNICAL_DESIGN.md section 8.3).

mod support;

use std::collections::BTreeMap;

use base::RepoPath;
use git::repository::{GitRepository, HashKind, SnapshotRepository, SourceEntry};
use support::TestRepo;

fn blob_ids(entries: &[SourceEntry]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|entry| (entry.path.to_string(), entry.blob_id.to_string()))
        .collect()
}

fn paths(entries: &[SourceEntry]) -> Vec<String> {
    entries.iter().map(|entry| entry.path.to_string()).collect()
}

#[test]
fn lists_only_supported_files_in_raw_path_byte_order() {
    let repo = TestRepo::init();
    repo.write("z.rs", "pub fn z() {}\n");
    repo.write("a.rs", "pub fn a() {}\n");
    repo.write("a/b.rs", "pub fn ab() {}\n");
    repo.write("a/c.rs", "pub fn ac() {}\n");
    repo.write("m.elm", "module M exposing (..)\n");
    repo.write("src/Main.elm", "module Main exposing (..)\n");
    repo.write("README.md", "# ignored\n");
    repo.write("data.json", "{}\n");
    repo.write("notes.txt", "ignored\n");
    repo.write("Cargo.toml", "[package]\n");
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");

    assert_eq!(
        paths(&entries),
        vec!["a.rs", "a/b.rs", "a/c.rs", "m.elm", "src/Main.elm", "z.rs",]
    );

    // `a.rs` sorts before `a/b.rs` because `.` (0x2E) precedes `/` (0x2F).
    for window in entries.windows(2) {
        assert!(
            window[0].path.as_bytes() <= window[1].path.as_bytes(),
            "entries are not sorted by raw path bytes"
        );
    }
}

#[test]
fn tracks_added_deleted_modified_and_unchanged_files() {
    let repo = TestRepo::init();
    repo.write("src/keep.rs", "pub fn keep() {}\n");
    repo.write("src/change.rs", "pub fn before() {}\n");
    repo.write("src/remove.rs", "pub fn removed() {}\n");
    let before = repo.commit("before");

    repo.write("src/change.rs", "pub fn after() {}\n");
    std::fs::remove_file(repo.path().join("src/remove.rs")).expect("remove file");
    repo.write("src/add.rs", "pub fn added() {}\n");
    let after = repo.commit("after");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let before_entries = discovered
        .source_entries(&discovered.resolve_commit(&before).expect("before"))
        .expect("enumerate before");
    let after_entries = discovered
        .source_entries(&discovered.resolve_commit(&after).expect("after"))
        .expect("enumerate after");

    assert_eq!(
        paths(&before_entries),
        vec!["src/change.rs", "src/keep.rs", "src/remove.rs"]
    );
    assert_eq!(
        paths(&after_entries),
        vec!["src/add.rs", "src/change.rs", "src/keep.rs"]
    );

    let before_ids = blob_ids(&before_entries);
    let after_ids = blob_ids(&after_entries);

    assert_ne!(
        before_ids["src/change.rs"], after_ids["src/change.rs"],
        "modified file must have a new blob id"
    );
    assert_eq!(
        before_ids["src/keep.rs"], after_ids["src/keep.rs"],
        "unchanged file must keep its blob id"
    );
    assert!(!after_ids.contains_key("src/remove.rs"));
    assert!(!before_ids.contains_key("src/add.rs"));
}

#[test]
fn records_executable_blobs() {
    let repo = TestRepo::init();
    repo.write("tool.rs", "pub fn tool() {}\n");
    repo.write("plain.rs", "pub fn plain() {}\n");
    repo.git_ok(&["add", "--all"]);
    repo.git_ok(&["update-index", "--chmod=+x", "tool.rs"]);
    repo.git_ok(&["commit", "-q", "-m", "executables"]);

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");

    let by_path: BTreeMap<String, bool> = entries
        .iter()
        .map(|entry| (entry.path.to_string(), entry.executable))
        .collect();
    assert!(by_path["tool.rs"]);
    assert!(!by_path["plain.rs"]);
}

#[test]
fn reads_packed_objects() {
    let repo = TestRepo::init();
    repo.write("src/a.rs", "pub fn a() {}\n");
    repo.write("src/b.elm", "module B exposing (..)\n");
    repo.commit("first");
    repo.write("src/a.rs", "pub fn a_changed() {}\n");
    repo.commit("second");

    repo.git_ok(&["repack", "-a", "-d"]);
    // `repack` alone leaves loose copies behind; `prune-packed` removes them so
    // the reads below must come from the pack.
    repo.git_ok(&["prune-packed"]);

    let pack_dir = repo.path().join(".git/objects/pack");
    let has_pack = std::fs::read_dir(&pack_dir)
        .expect("pack directory")
        .filter_map(Result::ok)
        .any(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"));
    assert!(has_pack, "repack should have produced a pack file");

    let objects_dir = repo.path().join(".git/objects");
    let loose = std::fs::read_dir(&objects_dir)
        .expect("objects directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.len() == 2 && name != "pack" && name != "info")
        })
        .flat_map(|entry| std::fs::read_dir(entry.path()).into_iter().flatten())
        .filter_map(Result::ok)
        .count();
    assert_eq!(loose, 0, "all objects should have been packed and pruned");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");
    assert_eq!(paths(&entries), vec!["src/a.rs", "src/b.elm"]);

    for entry in &entries {
        let bytes = discovered
            .read_blob(&entry.blob_id)
            .expect("read packed blob");
        assert!(!bytes.is_empty());
    }
}

#[test]
fn enumerates_a_sha256_repository_when_supported() {
    let Some(repo) = TestRepo::init_sha256() else {
        eprintln!("skipping SHA-256 test: the Git executable cannot create SHA-256 repositories");
        return;
    };

    repo.write("src/lib.rs", "pub fn sha256() {}\n");
    repo.write("src/Main.elm", "module Main exposing (..)\n");
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    assert_eq!(revision.object_id.kind, HashKind::Sha256);
    assert_eq!(revision.object_id.bytes.len(), 32);

    let entries = discovered.source_entries(&revision).expect("enumerate");
    assert_eq!(paths(&entries), vec!["src/Main.elm", "src/lib.rs"]);

    let blob = discovered
        .read_blob(&entries[1].blob_id)
        .expect("read sha256 blob");
    assert_eq!(blob, b"pub fn sha256() {}\n");
}

#[test]
fn enumerates_a_bare_repository() {
    let source = TestRepo::init();
    source.write("src/lib.rs", "pub fn bare() {}\n");
    source.write("src/Main.elm", "module Main exposing (..)\n");
    source.commit("initial");

    let bare = source.child("bare.git");
    source.git_ok(&[
        "clone",
        "-q",
        "--bare",
        source.path().to_str().expect("utf-8 path"),
        bare.to_str().expect("utf-8 path"),
    ]);

    let discovered = GitRepository::discover(&bare).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");
    assert_eq!(paths(&entries), vec!["src/Main.elm", "src/lib.rs"]);

    let blob = discovered
        .read_blob(&entries[1].blob_id)
        .expect("read bare blob");
    assert_eq!(blob, b"pub fn bare() {}\n");
}

#[cfg(unix)]
#[test]
fn ignores_symlinks_and_gitlinks() {
    let repo = TestRepo::init();
    repo.write("src/target.rs", "pub fn target() {}\n");
    std::os::unix::fs::symlink("target.rs", repo.path().join("src/link.rs")).expect("symlink");
    repo.git_ok(&["add", "--all"]);
    repo.git_ok(&["commit", "-q", "-m", "base"]);
    let base = repo.head_id();

    repo.git_ok(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{base},vendor/dep"),
    ]);
    repo.git_ok(&["commit", "-q", "-m", "add gitlink"]);

    let listing = repo.git_ok(&["ls-tree", "-r", "HEAD"]);
    assert!(
        listing.contains("120000 blob"),
        "symlink missing: {listing}"
    );
    assert!(
        listing.contains("160000 commit"),
        "gitlink missing: {listing}"
    );

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");

    assert_eq!(paths(&entries), vec!["src/target.rs"]);
}

#[cfg(unix)]
#[test]
fn sorts_invalid_utf8_paths_by_raw_bytes() {
    let repo = TestRepo::init();
    repo.write("src/a.rs", "pub fn a() {}\n");
    repo.git_ok(&["add", "--all"]);
    repo.git_ok(&["commit", "-q", "-m", "base"]);
    let blob = repo.rev_parse("HEAD:src/a.rs");

    let mut path = b"src/".to_vec();
    path.push(0xFF);
    path.extend_from_slice(b".rs");
    let mut cacheinfo = format!("100644,{blob},").into_bytes();
    cacheinfo.extend_from_slice(&path);

    repo.git_bytes(&[
        b"update-index",
        b"--add",
        b"--cacheinfo",
        cacheinfo.as_slice(),
    ]);
    repo.git_ok(&["commit", "-q", "-m", "invalid utf-8 path"]);

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");
    let entries = discovered.source_entries(&revision).expect("enumerate");

    let invalid = entries
        .iter()
        .find(|entry| entry.path.as_bytes().contains(&0xFF))
        .expect("the invalid-UTF-8 path should be retained");
    assert!(
        invalid.path.to_string().contains("\\xFF"),
        "Display must escape invalid UTF-8: {}",
        invalid.path
    );

    for window in entries.windows(2) {
        assert!(
            window[0].path.as_bytes() <= window[1].path.as_bytes(),
            "entries are not sorted by raw path bytes"
        );
    }
}

#[test]
fn reports_existing_file_and_directory_paths() {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", "pub fn lib() {}\n");
    repo.write("src/nested/deep.rs", "pub fn deep() {}\n");
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");

    for raw in ["src/lib.rs", "src", "src/nested", "src/nested/deep.rs"] {
        let path = RepoPath::new(raw).expect("valid repository path");
        assert!(
            discovered.path_exists(&revision, &path).expect("lookup"),
            "`{raw}` should exist"
        );
    }
}

#[test]
fn rejects_missing_and_shared_prefix_paths() {
    let repo = TestRepo::init();
    repo.write("src2/other.rs", "pub fn other() {}\n");
    repo.commit("initial");

    let discovered = GitRepository::discover(repo.path()).expect("discover");
    let revision = discovered.resolve_commit("HEAD").expect("HEAD");

    // `src` is absent even though the sibling `src2` exists, and a component
    // below a blob cannot resolve.
    for raw in [
        "src",
        "src/missing.rs",
        "src2/missing.rs",
        "src2/other.rs/deep",
    ] {
        let path = RepoPath::new(raw).expect("valid repository path");
        assert!(
            !discovered.path_exists(&revision, &path).expect("lookup"),
            "`{raw}` should not exist"
        );
    }
}
