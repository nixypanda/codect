//! A blocking mock HTTP server for the TypeSafe provider contract tests.
//!
//! It binds `127.0.0.1:0`, records each request's method, path, headers, and
//! body, and replies from a small script. It is built on `tiny_http` and never
//! uses an async runtime, so the tests exercise the same synchronous boundary
//! the provider implements.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tiny_http::{Header, Response, Server};

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

    /// Sets the response body.
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Adds one response header.
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
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
