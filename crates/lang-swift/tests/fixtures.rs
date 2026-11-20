// Expected projections are plain text so a failure shows a readable diff.
// Goldens are transcribed from the frozen contract (§8 and §11 of
// docs/plans/swift-language-contract.md), not captured from implementation output.

use std::fs;
use std::path::PathBuf;

use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_swift::SwiftProjector;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/swift")
}

fn fixture_cases() -> Vec<PathBuf> {
    let root = fixture_root();
    let mut cases: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("read {}: {error}", root.display()))
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    cases
}

#[test]
fn every_swift_fixture_case_contains_its_inputs_and_expectations() {
    let cases = fixture_cases();
    assert!(
        !cases.is_empty(),
        "expected at least one Swift fixture case"
    );

    for case in cases {
        for file in ["input.swift", "types.txt", "signatures.txt"] {
            assert!(
                case.join(file).is_file(),
                "fixture {case:?} is missing {file}"
            );
        }
    }
}

#[test]
fn swift_fixtures_match_expected_projections() {
    let projector = SwiftProjector::new();
    let cases = fixture_cases();
    assert!(
        !cases.is_empty(),
        "expected at least one Swift fixture case"
    );

    for case in cases {
        let name = case.file_name().expect("case name").to_string_lossy();
        let input = case.join("input.swift");
        let source = fs::read_to_string(&input)
            .unwrap_or_else(|error| panic!("read {}: {error}", input.display()));
        let relative = format!("fixtures/swift/{name}/input.swift");
        let path = SupportedPath::new(RepoPath::new(relative.as_bytes()).expect("fixture path"))
            .expect("fixture path has a supported extension");

        let mut expectations = vec![
            (ProjectionMode::Types, "types.txt"),
            (ProjectionMode::Signatures, "signatures.txt"),
        ];
        // A case commits to Tests mode by shipping a non-empty `tests.txt`.
        let tests_expected = case.join("tests.txt");
        if tests_expected.exists() {
            let expected = fs::read_to_string(&tests_expected)
                .unwrap_or_else(|e| panic!("read {}: {e}", tests_expected.display()));
            assert!(
                !expected.is_empty(),
                "fixture {name} ships an empty tests.txt (a broken detector also yields empty text)"
            );
            expectations.push((ProjectionMode::Tests, "tests.txt"));
        }

        for (mode, expected_file) in expectations {
            let expected_path = case.join(expected_file);
            let expected = fs::read_to_string(&expected_path)
                .unwrap_or_else(|error| panic!("read {}: {error}", expected_path.display()));
            let projected = projector
                .project(ProjectionInput {
                    path: &path,
                    source: &source,
                    mode,
                })
                .unwrap_or_else(|error| panic!("{relative} {mode:?}: {error}"));

            assert_eq!(
                projected.canonical_text(),
                expected,
                "{relative} did not match {expected_file}"
            );
        }
    }
}
