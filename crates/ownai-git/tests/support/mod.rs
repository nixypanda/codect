//! Shared fixtures for `ownai-git` integration tests.
//!
//! # Why the `git` executable appears here
//!
//! `ownai-git` is read-only and never invokes the Git executable. The approved
//! `gix` feature set (TECHNICAL_DESIGN.md section 4.1) cannot create commits or
//! references, so tests need real repositories. Every `git` invocation in this
//! module is **test setup only** and is isolated from host configuration and
//! network access.
//!
//! Tests never call `git` after fixtures are built: they exercise only
//! `GitRepository` and `SnapshotRepository`.
//!
//! Environment isolation:
//!
//! - `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` point at `/dev/null`.
//! - `GIT_CONFIG_NOSYSTEM` disables the system configuration.
//! - Author, committer, and date are fixed so commit ids are reproducible.
//! - Prompting and pagers are disabled so a test can never hang.

#![allow(dead_code)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

/// A fixed commit date so generated commit ids are reproducible.
const FIXED_DATE: &str = "2020-01-01T00:00:00+0000";

pub struct TestRepo {
    root: TempDir,
}

impl TestRepo {
    pub fn new() -> Self {
        Self {
            root: TempDir::new().expect("temporary directory"),
        }
    }

    pub fn init() -> Self {
        let repo = Self::new();
        repo.git_ok(&["init", "-q"]);
        repo
    }

    pub fn init_bare() -> Self {
        let repo = Self::new();
        repo.git_ok(&["init", "-q", "--bare"]);
        repo
    }

    /// Creates a SHA-256 repository, or `None` when the environment's Git
    /// cannot create one.
    pub fn init_sha256() -> Option<Self> {
        let repo = Self::new();
        let output = repo.git(&["init", "-q", "--object-format=sha256"]);
        if !output.status.success() {
            return None;
        }
        // Confirm the repository really uses SHA-256 rather than silently
        // falling back to SHA-1.
        let format = repo.git_ok(&["rev-parse", "--show-object-format"]);
        (format.trim() == "sha256").then_some(repo)
    }

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn child(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    pub fn git(&self, args: &[&str]) -> Output {
        git(self.path(), args)
    }

    pub fn git_ok(&self, args: &[&str]) -> String {
        git_ok(self.path(), args)
    }

    pub fn git_bytes(&self, args: &[&[u8]]) -> Output {
        git_bytes(self.path(), args)
    }

    pub fn write(&self, rel: &str, contents: &str) {
        write(self.path(), rel, contents.as_bytes());
    }

    pub fn write_bytes(&self, rel: &str, contents: &[u8]) {
        write(self.path(), rel, contents);
    }

    pub fn add_all(&self) {
        self.git_ok(&["add", "--all"]);
    }

    pub fn commit(&self, message: &str) -> String {
        self.git_ok(&["add", "--all"]);
        self.git_ok(&["commit", "-q", "--allow-empty", "-m", message]);
        self.head_id()
    }

    pub fn commit_empty(&self, message: &str) -> String {
        self.git_ok(&["commit", "-q", "--allow-empty", "-m", message]);
        self.head_id()
    }

    pub fn head_id(&self) -> String {
        self.git_ok(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    pub fn rev_parse(&self, spec: &str) -> String {
        self.git_ok(&["rev-parse", spec]).trim().to_owned()
    }

    pub fn tag_annotated(&self, name: &str, message: &str) {
        self.git_ok(&["tag", "-a", name, "-m", message]);
    }
}

pub fn git(dir: &Path, args: &[&str]) -> Output {
    git_bytes(
        dir,
        &args.iter().map(|arg| arg.as_bytes()).collect::<Vec<_>>(),
    )
}

/// Runs `git` in `dir`, isolated from host configuration and network access.
pub fn git_bytes(dir: &Path, args: &[&[u8]]) -> Output {
    let mut command = Command::new("git");
    for prefix in [
        "user.name=OwnAI Test",
        "user.email=ownai@example.invalid",
        "init.defaultBranch=main",
        "commit.gpgsign=false",
        "tag.gpgsign=false",
        "core.autocrlf=false",
    ] {
        command.arg("-c").arg(prefix);
    }
    command.args(args.iter().map(|arg| OsStr::from_bytes(arg)));
    command
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_AUTHOR_NAME", "OwnAI Test")
        .env("GIT_AUTHOR_EMAIL", "ownai@example.invalid")
        .env("GIT_COMMITTER_NAME", "OwnAI Test")
        .env("GIT_COMMITTER_EMAIL", "ownai@example.invalid")
        .env("GIT_AUTHOR_DATE", FIXED_DATE)
        .env("GIT_COMMITTER_DATE", FIXED_DATE)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("the `git` executable must be available for test setup")
}

pub fn git_ok(dir: &Path, args: &[&str]) -> String {
    let output = git(dir, args);
    assert!(
        output.status.success(),
        "`git {}` failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout is UTF-8 in tests")
}

pub fn write(dir: &Path, rel: &str, contents: &[u8]) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directories");
    }
    std::fs::write(&path, contents).expect("write fixture file");
}

/// Used to build a genuinely ambiguous abbreviated revision.
pub fn find_ambiguous_prefix(dir: &Path) -> Option<String> {
    let listing = git_ok(dir, &["cat-file", "--batch-check", "--batch-all-objects"]);
    let mut seen = std::collections::HashMap::<String, String>::new();
    for line in listing.lines() {
        let Some(id) = line.split_whitespace().next() else {
            continue;
        };
        if id.len() < 4 {
            continue;
        }
        let prefix = id[..4].to_owned();
        if seen.contains_key(&prefix) {
            return Some(prefix);
        }
        seen.insert(prefix, id.to_owned());
    }
    None
}
