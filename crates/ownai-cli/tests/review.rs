//! End-to-end tests for `diff --lens review` with the deterministic fake
//! provider (TECHNICAL_DESIGN.md sections 11.3, 15).
//!
//! Every assertion runs against a real temporary Git repository through the
//! built binary; the fake provider makes the review reproducible and network
//! free.

mod support;

use std::process::Output;

use support::{TestRepo, ownai_in, stderr, stdout};

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

/// The canonical diff of the widened signature in signatures mode. The plain
/// and review documents embed it verbatim.
const CANONICAL_DIFF: &str = concat!(
    "diff --ownai a/src/lib.rs b/src/lib.rs\n",
    "--- a/src/lib.rs\n",
    "+++ b/src/lib.rs\n",
    "@@ -2,4 +2,4 @@\n",
    "     pub id: u32,\n",
    " }\n",
    " \n",
    "-pub fn greet(name: &str) -> String;\n",
    "+pub fn greet(name: &str, excited: bool) -> String;\n",
);

const REVIEW_HEADER: &str = concat!(
    "ownai review\n",
    "provider: fake\n",
    "model-revision: fake-v1\n",
    "lens: review\n",
    "state-schema: ownai.review-unit.v1\n",
    "units: 1 reviewed, 0 skipped, 0 failed\n",
    "\n",
);

/// The projector's stable key qualifies a top-level declaration with its file
/// path and a kind token, so the widened `greet` renders as
/// `src/lib.rs::fn::greet`.
const REVIEW_ANNOTATION: &str = concat!(
    "src/lib.rs :: src/lib.rs::fn::greet\n",
    "  change: modified\n",
    "  concern: api-contract (confidence 0.90)\n",
    "  risk: 1.0/5 (confidence 0.50)\n",
    "  likely-breaking: 0.50\n",
    "  needs-tests: 0.50\n",
    "  needs-docs: 0.50\n",
    "  needs-migration: 0.50\n",
    "  security-sensitive: 0.50\n",
);

/// A repository whose second commit widens the `greet` signature, modifying a
/// top-level declaration in signatures mode.
fn repo_with_widened_signature() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("src/lib.rs", RUST_BASE);
    repo.commit("base");
    repo.write("src/lib.rs", RUST_WIDENED);
    repo.commit("widen signature");
    repo
}

fn run(repo: &TestRepo, args: &[&str]) -> Output {
    ownai_in(repo, args).output().expect("run ownai")
}

#[test]
fn review_document_with_the_fake_provider_is_exact() {
    let repo = repo_with_widened_signature();
    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--lens",
            "review",
            "--decision-provider",
            "fake",
            "HEAD~1",
            "HEAD",
        ],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        output.stderr.is_empty(),
        "a successful review must not write diagnostics"
    );

    let expected = format!("{REVIEW_HEADER}{CANONICAL_DIFF}\n{REVIEW_ANNOTATION}");
    assert_eq!(stdout(&output), expected);
}

#[test]
fn decision_model_override_appears_in_the_header() {
    let repo = repo_with_widened_signature();
    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--lens",
            "review",
            "--decision-provider",
            "fake",
            "--decision-model",
            "custom-model",
            "HEAD~1",
            "HEAD",
        ],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(output.stderr.is_empty());
    assert!(stdout(&output).contains("model-revision: custom-model\n"));
}

#[test]
fn plain_diff_without_a_lens_is_unchanged() {
    let repo = repo_with_widened_signature();
    let output = run(&repo, &["diff", "--mode", "signatures", "HEAD~1", "HEAD"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(output.stderr.is_empty());

    let document = stdout(&output);
    assert!(
        !document.contains("ownai review"),
        "a plain diff must not contain a review document"
    );
    assert_eq!(document, CANONICAL_DIFF);
}

#[test]
fn unavailable_providers_exit_non_zero_with_a_diagnostic() {
    let repo = repo_with_widened_signature();
    for provider in ["typesafe", "laya"] {
        let output = run(
            &repo,
            &[
                "diff",
                "--mode",
                "signatures",
                "--lens",
                "review",
                "--decision-provider",
                provider,
                "HEAD~1",
                "HEAD",
            ],
        );

        assert!(!output.status.success(), "{provider} must not succeed");
        assert!(
            output.stdout.is_empty(),
            "{provider} must not write a review document"
        );
        assert!(!output.stderr.is_empty(), "{provider} must report why");
    }
}

#[test]
fn a_lens_without_a_provider_is_a_usage_error() {
    let repo = repo_with_widened_signature();
    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--lens",
            "review",
            "HEAD~1",
            "HEAD",
        ],
    );

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(!output.stderr.is_empty(), "clap must print usage");
}

#[test]
fn a_provider_without_a_lens_is_a_usage_error() {
    let repo = repo_with_widened_signature();
    let output = run(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--decision-provider",
            "fake",
            "HEAD~1",
            "HEAD",
        ],
    );

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(!output.stderr.is_empty(), "clap must print usage");
}
