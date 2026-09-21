//! Trimmed fixture helpers for the `ownai-cli` end-to-end tests.
//!
//! # Why the `git` executable appears here
//!
//! `ownai-cli` and `ownai-git` are read-only and never invoke Git. The approved
//! `gix` feature set (TECHNICAL_DESIGN.md section 4.1) cannot create commits or
//! references, so the tests build real repositories with the `git` executable.
//! Every invocation is test setup only and is isolated from host configuration
//! and the network, exactly as `ownai-git`'s support module does:
//!
//! - `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` point at `/dev/null`.
//! - `GIT_CONFIG_NOSYSTEM` disables the system configuration.
//! - Author, committer, and date are fixed so commit ids are reproducible.
//! - Prompting and signing are disabled so a test can never hang or fail on a
//!   developer's Git configuration.

#![allow(dead_code)]

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

    pub fn path(&self) -> &Path {
        self.root.path()
    }

    pub fn write(&self, rel: &str, contents: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directories");
        }
        std::fs::write(&path, contents).expect("write fixture file");
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.path().join(rel)).expect("remove fixture file");
    }

    pub fn git(&self, args: &[&str]) -> Output {
        git(self.path(), args)
    }

    pub fn git_ok(&self, args: &[&str]) -> String {
        git_ok(self.path(), args)
    }

    pub fn commit(&self, message: &str) -> String {
        self.git_ok(&["add", "--all"]);
        self.git_ok(&["commit", "-q", "--allow-empty", "-m", message]);
        self.head_id()
    }

    pub fn head_id(&self) -> String {
        self.git_ok(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// Clones this repository into a fresh bare repository. `gix` must be able
    /// to discover and read the clone directly from its root.
    pub fn clone_bare(&self) -> Self {
        let bare = Self::new();
        let source = self.path().to_string_lossy().into_owned();
        let destination = bare.path().to_string_lossy().into_owned();
        git_ok(
            self.path(),
            &["clone", "-q", "--bare", &source, &destination],
        );
        bare
    }
}

/// Builds the `ownai` binary invocation with Git repository-override variables
/// removed, so a developer's environment cannot redirect discovery.
pub fn ownai() -> assert_cmd::Command {
    let mut command = assert_cmd::Command::cargo_bin("ownai").expect("ownai binary is built");
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    command
}

pub fn ownai_in(repo: &TestRepo, args: &[&str]) -> assert_cmd::Command {
    let mut command = ownai();
    command.current_dir(repo.path()).args(args);
    command
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8 in tests")
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8 in tests")
}

/// Reads an expected projection from the repository-level `fixtures/` tree.
pub fn fixture(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read fixture {}: {error}", path.display()))
}

fn git(dir: &Path, args: &[&str]) -> Output {
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
    command.args(args);
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

fn git_ok(dir: &Path, args: &[&str]) -> String {
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
