// Trimmed fixture helpers for the `engine` integration tests.
//
// The engine and `git` are read-only and never invoke Git. The approved
// `gix` feature set cannot create commits or references, so tests build real
// repositories with the `git` executable. Every invocation is test setup only
// and is isolated from host configuration and the network.

#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

// A fixed commit date so generated commit ids are reproducible.
const FIXED_DATE: &str = "2020-01-01T00:00:00+0000";

pub struct TestRepo {
    root: TempDir,
}

impl TestRepo {
    pub fn init() -> Self {
        let repo = Self {
            root: TempDir::new().expect("temporary directory"),
        };
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

    pub fn commit(&self, message: &str) -> String {
        self.git_ok(&["add", "--all"]);
        self.git_ok(&["commit", "-q", "--allow-empty", "-m", message]);
        self.head_id()
    }

    pub fn head_id(&self) -> String {
        self.git_ok(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    pub fn clone_bare(&self) -> Self {
        let bare = Self {
            root: TempDir::new().expect("temporary directory"),
        };
        let source = self.path().to_string_lossy().into_owned();
        let destination = bare.path().to_string_lossy().into_owned();
        git_ok(
            self.path(),
            &["clone", "-q", "--bare", &source, &destination],
        );
        bare
    }

    fn git_ok(&self, args: &[&str]) -> String {
        let output = git(self.path(), args);
        assert!(
            output.status.success(),
            "`git {}` failed in {}: {}",
            args.join(" "),
            self.path().display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("git stdout is UTF-8 in tests")
    }
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

// The repository-relative path of a test file, for building selections.
pub fn repo_path(raw: &str) -> base::RepoPath {
    base::RepoPath::new(raw).expect("valid repository path")
}
