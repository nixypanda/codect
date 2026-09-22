//! End-to-end tests for the hidden `eval-lens` evaluation harness.
//!
//! The harness reads a checked-in corpus and uses the deterministic fake
//! provider, so these tests are reproducible and never touch the network. They
//! run outside a Git repository on purpose: `eval-lens` must not require one.

mod support;

use std::path::PathBuf;
use std::process::Output;

use support::{TestRepo, ownai_in, stderr, stdout};

fn fixtures_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/eval/review-v1.json")
        .to_string_lossy()
        .into_owned()
}

/// Runs `ownai` in a plain temporary directory, not a Git repository.
fn run(args: &[&str]) -> Output {
    let dir = TestRepo::new();
    ownai_in(&dir, args).output().expect("run ownai")
}

#[test]
fn eval_lens_with_the_fake_provider_prints_the_report() {
    let output = run(&[
        "eval-lens",
        "--fixtures",
        &fixtures_path(),
        "--decision-provider",
        "fake",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        output.stderr.is_empty(),
        "a successful evaluation must not write diagnostics: {}",
        stderr(&output)
    );

    let document = stdout(&output);
    assert!(
        document.starts_with("ownai review evaluation\n"),
        "{document}"
    );
    assert!(
        document.contains("schema: ownai.review-eval.v1\n"),
        "{document}"
    );
    assert!(
        document.contains("choice-confidence-threshold: 0.55\n"),
        "{document}"
    );
    assert!(document.contains("cases: 18\n"), "{document}");
    assert!(document.contains("labeled: 17\n"), "{document}");
    assert!(document.contains("ambiguous: 1\n"), "{document}");
    assert!(document.contains("failed: 0\n"), "{document}");
    assert!(document.contains("noul-brier: 0.2500\n"), "{document}");

    assert!(document.contains("\nby language:\n"), "{document}");
    assert!(document.contains("\nby change:\n"), "{document}");
    assert!(document.contains("\nby item kind:\n"), "{document}");
    assert!(document.contains("  elm: cases="), "{document}");
    assert!(document.contains("  rust: cases="), "{document}");
    assert!(document.contains("  haskell: cases="), "{document}");
    assert!(document.contains("  python: cases="), "{document}");

    assert!(document.ends_with('\n'), "{document}");
    assert!(!document.ends_with("\n\n"), "{document}");
}

#[test]
fn eval_lens_with_a_missing_fixtures_file_fails_with_an_empty_stdout() {
    let output = run(&[
        "eval-lens",
        "--fixtures",
        "/nonexistent/ownai/review-v1.json",
        "--decision-provider",
        "fake",
    ]);

    assert!(!output.status.success(), "a missing file must fail");
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("evaluation fixtures"),
        "the diagnostic must name the fixtures: {}",
        stderr(&output)
    );
}

#[test]
fn eval_lens_with_an_unknown_schema_fails_with_a_diagnostic() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let path = dir.path().join("review.json");
    std::fs::write(&path, r#"{"schema": "ownai.review-eval.v2", "cases": []}"#)
        .expect("write fixture");
    let path = path.to_string_lossy().into_owned();

    let output = run(&[
        "eval-lens",
        "--fixtures",
        &path,
        "--decision-provider",
        "fake",
    ]);

    assert!(!output.status.success(), "an unknown schema must fail");
    assert!(output.stdout.is_empty(), "stdout must stay empty");
    assert!(
        stderr(&output).contains("schema"),
        "the diagnostic must name the schema: {}",
        stderr(&output)
    );
}

#[test]
fn eval_lens_accepts_a_threshold_override() {
    let output = run(&[
        "eval-lens",
        "--fixtures",
        &fixtures_path(),
        "--decision-provider",
        "fake",
        "--choice-confidence-threshold",
        "0.9",
    ]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("choice-confidence-threshold: 0.9\n"),
        "{}",
        stdout(&output)
    );
}
