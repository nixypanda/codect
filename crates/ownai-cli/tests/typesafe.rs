//! End-to-end tests for the TypeSafe decision provider against a local mock
//! HTTP server. They cover the CLI contract: the disclosure gate, credential
//! resolution, custom-endpoint opt-in, and the review and evaluation documents.

#![cfg(feature = "typesafe")]

mod support;

use std::path::PathBuf;
use std::process::Output;

use serde_json::json;
use support::{MockServer, ScriptedResponse, TestRepo, ownai_in, stderr, stdout};

const RUST_BASE: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}
";

const RUST_WIDENED: &str = "\
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str, excited: bool) -> String {
    format!(\"hi {name}\")
}
";

/// A repository whose second commit widens the `greet` signature.
fn repo_with_widened_signature() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_WIDENED);
    repo.commit("widen signature");
    repo
}

/// A response that is valid for the fixed review lens question set. The same
/// body answers every unit, so an evaluation run can reuse one script.
fn review_response() -> String {
    json!({
        "provider": "typesafe",
        "model_revision": "system-one-mock-1",
        "answers": {
            "concern": {
                "type": "choice",
                "choice": "api-contract",
                "probabilities": {
                    "api-contract": 0.55,
                    "data-model": 0.05,
                    "authentication-authorization": 0.05,
                    "storage": 0.05,
                    "networking-integration": 0.05,
                    "configuration-operations": 0.05,
                    "observability": 0.05,
                    "user-interface": 0.05,
                    "testing-tooling": 0.05,
                    "other": 0.05
                },
                "confidence": 0.8
            },
            "risk": {
                "type": "score",
                "score": 1.0,
                "probabilities": {
                    "routine": 0.2,
                    "low": 0.2,
                    "moderate": 0.2,
                    "high": 0.2,
                    "critical": 0.2
                },
                "confidence": 0.7
            },
            "likely_breaking": { "type": "noul", "noul": 0.6 },
            "needs_tests": { "type": "noul", "noul": 0.6 },
            "needs_docs": { "type": "noul", "noul": 0.6 },
            "needs_migration": { "type": "noul", "noul": 0.6 },
            "security_sensitive": { "type": "noul", "noul": 0.6 }
        },
        "usage": { "input_tokens": 50, "output_tokens": 10 }
    })
    .to_string()
}

fn fixtures_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/eval/review-v1.json")
        .to_string_lossy()
        .into_owned()
}

fn diff_args(endpoint: &str) -> Vec<String> {
    [
        "diff",
        "--mode",
        "signatures",
        "--lens",
        "review",
        "--decision-provider",
        "typesafe",
        "--decision-endpoint",
        endpoint,
        "--allow-custom-endpoint",
        "--accept-disclosure",
        "HEAD~1",
        "HEAD",
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect()
}

fn run(repo: &TestRepo, args: &[String], key: Option<&str>) -> Output {
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let mut command = ownai_in(repo, &argv);
    command
        .env_remove("OWNAI_ACCEPT_DISCLOSURE")
        .env_remove("OWNAI_DECISION_MODEL")
        .env_remove("TYPESAFE_API_KEY");
    if let Some(key) = key {
        command.env("TYPESAFE_API_KEY", key);
    }
    command.output().expect("run ownai")
}

#[test]
fn a_diff_review_with_typesafe_writes_a_document_and_discloses() {
    let repo = repo_with_widened_signature();
    let server = MockServer::scripted(vec![ScriptedResponse::json(review_response())]);
    let endpoint = server.base_url();

    let output = run(&repo, &diff_args(&endpoint), Some("test-key"));

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains("ownai review disclosure\n"),
        "the disclosure must be written to stderr: {diagnostic}"
    );
    assert!(diagnostic.contains("provider: typesafe\n"), "{diagnostic}");
    assert!(diagnostic.contains("route: remote\n"), "{diagnostic}");
    assert!(diagnostic.contains("host: 127.0.0.1\n"), "{diagnostic}");

    let document = stdout(&output);
    assert!(document.contains("provider: typesafe\n"), "{document}");
    assert!(
        document.contains("model-revision: system-one-mock-1\n"),
        "{document}"
    );
    assert!(document.contains("concern: api-contract"), "{document}");

    let recorded = server.requests();
    assert_eq!(recorded.len(), 1, "one changed declaration is one request");
    assert_eq!(recorded[0].header("authorization"), Some("Bearer test-key"));
}

#[test]
fn typesafe_without_a_key_fails_with_an_empty_stdout() {
    let repo = repo_with_widened_signature();
    let server = MockServer::always(ScriptedResponse::json(review_response()));
    let endpoint = server.base_url();

    let output = run(&repo, &diff_args(&endpoint), None);

    assert!(!output.status.success(), "a missing key must fail");
    assert!(
        output.stdout.is_empty(),
        "no review document may be written: {}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("TYPESAFE_API_KEY"),
        "the diagnostic must name the variable: {}",
        stderr(&output)
    );
    assert!(
        server.requests().is_empty(),
        "no request may be sent without a credential"
    );
}

#[test]
fn an_explicit_endpoint_without_opt_in_fails_with_an_empty_stdout() {
    let repo = repo_with_widened_signature();
    let server = MockServer::always(ScriptedResponse::json(review_response()));
    let endpoint = server.base_url();

    let mut args = diff_args(&endpoint);
    // Drop the opt-in flag while keeping the explicit endpoint.
    args.retain(|value| value != "--allow-custom-endpoint");

    let output = run(&repo, &args, Some("test-key"));

    assert!(!output.status.success(), "the opt-in is required");
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("allow-custom-endpoint"),
        "the diagnostic must explain the opt-in: {}",
        stderr(&output)
    );
    assert!(server.requests().is_empty(), "no request may be sent");
}

#[test]
fn eval_lens_with_typesafe_prints_a_report() {
    let dir = TestRepo::new();
    let server = MockServer::always(ScriptedResponse::json(review_response()));
    let endpoint = server.base_url();
    let fixtures = fixtures_path();

    let output = run(
        &dir,
        &[
            "eval-lens".to_owned(),
            "--fixtures".to_owned(),
            fixtures,
            "--decision-provider".to_owned(),
            "typesafe".to_owned(),
            "--decision-endpoint".to_owned(),
            endpoint,
            "--allow-custom-endpoint".to_owned(),
        ],
        Some("test-key"),
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let report = stdout(&output);
    assert!(report.starts_with("ownai review evaluation\n"), "{report}");
    assert!(report.contains("cases: 18\n"), "{report}");
    assert!(report.contains("failed: 0\n"), "{report}");
    assert_eq!(server.requests().len(), 18, "one request per case");
}
