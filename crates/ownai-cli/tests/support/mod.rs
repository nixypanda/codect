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

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tiny_http::{Header, Response, Server};

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

    /// Creates a SHA-256 repository, or `None` when the environment's Git
    /// cannot create one. Mirrors `ownai-git`'s support helper so the CLI is
    /// covered end to end with the same runtime capability check.
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
        .env_remove("GIT_INDEX_FILE")
        // Semantic-lens settings must never leak from a developer's
        // environment into an assertion; tests that need them set them
        // explicitly on the returned command.
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("OWNAI_DECISION_MODEL")
        .env_remove("OWNAI_ACCEPT_DISCLOSURE");
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

// ---------------------------------------------------------------------------
// A loopback mock HTTP server for the TypeSafe provider end-to-end tests.
// ---------------------------------------------------------------------------

/// One recorded request.
#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl RecordedRequest {
    /// The value of a header, matched case-insensitively.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// One scripted reply.
#[derive(Clone, Debug)]
pub struct ScriptedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl ScriptedResponse {
    /// A `200` reply with the given body.
    pub fn json(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// A reply with the given status and an empty body.
    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: String::new(),
        }
    }
}

/// How the server replies to requests.
enum Script {
    /// Replies in order, then `500` once exhausted.
    Once(VecDeque<ScriptedResponse>),
    /// Replies with the same response for every request.
    Always(ScriptedResponse),
}

/// A loopback HTTP server that records requests and replies from a script.
pub struct MockServer {
    addr: SocketAddr,
    recorded: Arc<Mutex<Vec<RecordedRequest>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Starts a server that replies to each request with the next scripted
    /// response, then `500`.
    pub fn scripted(responses: Vec<ScriptedResponse>) -> Self {
        Self::start(Script::Once(responses.into()))
    }

    /// Starts a server that replies to every request with `response`.
    pub fn always(response: ScriptedResponse) -> Self {
        Self::start(Script::Always(response))
    }

    fn start(script: Script) -> Self {
        let server = Server::http("127.0.0.1:0").expect("bind a loopback port");
        let addr = server
            .server_addr()
            .to_ip()
            .expect("the mock server listens on an IP socket");
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_recorded = Arc::clone(&recorded);
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            let mut script = script;
            while !thread_shutdown.load(Ordering::Relaxed) {
                match server.recv_timeout(Duration::from_millis(20)) {
                    Ok(Some(mut request)) => {
                        let record = record(&mut request);
                        thread_recorded
                            .lock()
                            .expect("mock server request lock is not poisoned")
                            .push(record);
                        let reply = next_reply(&mut script);
                        let _ = request.respond(build_response(reply));
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            recorded,
            shutdown,
            handle: Some(handle),
        }
    }

    /// The `http://host:port` base URL of this server.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// A snapshot of every request received so far, in arrival order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.recorded
            .lock()
            .expect("mock server request lock is not poisoned")
            .clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Returns the next scripted reply.
fn next_reply(script: &mut Script) -> ScriptedResponse {
    match script {
        Script::Always(response) => response.clone(),
        Script::Once(queue) => queue
            .pop_front()
            .unwrap_or_else(|| ScriptedResponse::status(500)),
    }
}

/// Records one request before it is answered.
fn record(request: &mut tiny_http::Request) -> RecordedRequest {
    let method = request.method().to_string();
    let path = request.url().to_owned();
    let headers = request
        .headers()
        .iter()
        .map(|header| (header.field.to_string(), header.value.as_str().to_owned()))
        .collect();
    let mut body = String::new();
    let _ = request.as_reader().read_to_string(&mut body);
    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}

/// Converts a scripted reply into a `tiny_http` response.
fn build_response(reply: ScriptedResponse) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut response = Response::from_string(reply.body).with_status_code(reply.status);
    for (name, value) in reply.headers {
        if let Ok(header) = Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(header);
        }
    }
    response
}
