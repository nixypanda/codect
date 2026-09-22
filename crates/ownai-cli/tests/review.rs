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

/// The exact review header for a repository with no `.ownai.toml`: the CLI uses
/// the built-in concern taxonomy, so the digest matches the built-in lens.
fn review_header() -> String {
    let digest = ownai_engine::ReviewLens::new(1 << 20)
        .expect("the built-in lens is valid")
        .question_digest()
        .to_owned();
    format!(
        concat!(
            "ownai review\n",
            "provider: fake\n",
            "model-revision: fake-v1\n",
            "lens: review\n",
            "state-schema: ownai.review-unit.v1\n",
            "question-set: {}\n",
            "units: 1 reviewed, 0 skipped, 0 failed\n",
            "\n",
        ),
        digest,
    )
}

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

/// Runs with any inherited acknowledgement removed, so the disclosure gate is
/// tested independently of the developer's environment.
fn run_unacknowledged(repo: &TestRepo, args: &[&str]) -> Output {
    ownai_in(repo, args)
        .env_remove("OWNAI_ACCEPT_DISCLOSURE")
        .output()
        .expect("run ownai")
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

    let expected = format!("{}{CANONICAL_DIFF}\n{REVIEW_ANNOTATION}", review_header());
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

/// The exact dry-run plan for the widened signature, with the serialized state
/// size pinned. No provider is built and no annotation is emitted.
const DRY_RUN_PLAN: &str = concat!(
    "ownai review dry run\n",
    "provider: fake\n",
    "route: local-test\n",
    "model: (provider default)\n",
    "units: 1\n",
    "serialized-state-bytes: 356\n",
    "skipped: 0\n",
    "paths:\n",
    "  src/lib.rs\n",
    "no request was sent\n",
);

#[test]
fn fake_dry_run_prints_the_plan_to_stdout_without_annotating() {
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
            "--dry-run",
            "HEAD~1",
            "HEAD",
        ],
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        output.stderr.is_empty(),
        "a dry run must not write to stderr: {}",
        stderr(&output)
    );

    let document = stdout(&output);
    assert_eq!(document, DRY_RUN_PLAN);
    assert!(
        !document.contains("change:") && !document.contains("concern:"),
        "a dry run must not emit review annotations: {document}"
    );
}

#[test]
fn remote_review_without_acknowledgement_is_refused_non_interactively() {
    let repo = repo_with_widened_signature();
    let output = run_unacknowledged(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--lens",
            "review",
            "--decision-provider",
            "typesafe",
            "HEAD~1",
            "HEAD",
        ],
    );

    assert!(!output.status.success(), "the gate must fail");
    assert!(
        output.stdout.is_empty(),
        "no state may be transmitted or rendered: {}",
        stdout(&output)
    );
    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains("acknowledg"),
        "stderr must explain the acknowledgement requirement: {diagnostic}"
    );
}

#[test]
fn accept_disclosure_flag_passes_the_remote_gate() {
    let repo = repo_with_widened_signature();
    let output = ownai_in(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--lens",
            "review",
            "--decision-provider",
            "typesafe",
            "--accept-disclosure",
            "HEAD~1",
            "HEAD",
        ],
    )
    .env_remove("OWNAI_ACCEPT_DISCLOSURE")
    .output()
    .expect("run ownai");

    assert!(
        !output.status.success(),
        "typesafe is not implemented yet, so the run still fails"
    );
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("provider: typesafe\n"),
        "the disclosure must name the provider: {}",
        stderr(&output)
    );
    assert!(stderr(&output).contains("route: remote\n"));
    assert!(stderr(&output).contains("host: (provider default)\n"));
    assert!(stderr(&output).contains("units:"));
}

#[test]
fn accept_disclosure_environment_passes_the_remote_gate() {
    let repo = repo_with_widened_signature();
    let output = ownai_in(
        &repo,
        &[
            "diff",
            "--mode",
            "signatures",
            "--lens",
            "review",
            "--decision-provider",
            "typesafe",
            "HEAD~1",
            "HEAD",
        ],
    )
    .env("OWNAI_ACCEPT_DISCLOSURE", "1")
    .output()
    .expect("run ownai");

    assert!(
        !output.status.success(),
        "typesafe is not implemented yet, so the run still fails"
    );
    let diagnostic = stderr(&output);
    assert!(diagnostic.contains("provider: typesafe\n"), "{diagnostic}");
    assert!(diagnostic.contains("route: remote\n"), "{diagnostic}");
    assert!(diagnostic.contains("units:"), "{diagnostic}");
}

#[test]
fn configured_threshold_and_concerns_change_the_concern_line_and_digest() {
    let repo = repo_with_widened_signature();
    repo.write(
        ".ownai.toml",
        concat!(
            "[lenses.review]\n",
            "choice_confidence_threshold = 0.95\n",
            "\n",
            "[lenses.review.concerns]\n",
            "engine = \"Projection, Git access, selection, and application behavior\"\n",
            "other = \"No listed concern is a good fit\"\n",
        ),
    );

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

    let document = stdout(&output);
    assert!(
        document.contains("  concern: uncertain (leading: engine 0.50, other 0.50)\n"),
        "the configured taxonomy and threshold must render uncertainty: {document}"
    );

    let concerns = ownai_engine::ReviewConcerns::new(vec![
        (
            "engine".to_owned(),
            "Projection, Git access, selection, and application behavior".to_owned(),
        ),
        (
            "other".to_owned(),
            "No listed concern is a good fit".to_owned(),
        ),
    ])
    .expect("valid concerns");
    let digest = ownai_engine::ReviewLens::with_concerns(1 << 20, concerns)
        .expect("valid lens")
        .question_digest()
        .to_owned();
    assert!(
        document.contains(&format!("question-set: {digest}\n")),
        "the header must carry the configured question-set digest: {document}"
    );
}

#[test]
fn a_concern_taxonomy_without_other_exits_non_zero_with_empty_stdout() {
    let repo = repo_with_widened_signature();
    repo.write(
        ".ownai.toml",
        "[lenses.review.concerns]\nengine = \"Engine behavior\"\n",
    );

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

    assert!(!output.status.success(), "a missing `other` must fail");
    assert!(
        output.stdout.is_empty(),
        "no document may be written: {}",
        stdout(&output)
    );
    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains("review concerns"),
        "stderr must explain the invalid concerns: {diagnostic}"
    );
}

#[test]
fn an_out_of_range_threshold_exits_non_zero_with_empty_stdout() {
    let repo = repo_with_widened_signature();
    repo.write(
        ".ownai.toml",
        "[lenses.review]\nchoice_confidence_threshold = 1.5\n",
    );

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

    assert!(!output.status.success(), "an invalid threshold must fail");
    assert!(
        output.stdout.is_empty(),
        "no document may be written: {}",
        stdout(&output)
    );
    let diagnostic = stderr(&output);
    assert!(
        diagnostic.contains("threshold"),
        "stderr must explain the invalid threshold: {diagnostic}"
    );
}

#[test]
fn a_malformed_config_does_not_break_a_plain_diff_without_a_lens() {
    let repo = repo_with_widened_signature();
    repo.write(".ownai.toml", "this is not toml\n");

    let output = run(&repo, &["diff", "--mode", "signatures", "HEAD~1", "HEAD"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(output.stderr.is_empty());
    assert_eq!(stdout(&output), CANONICAL_DIFF);
}
