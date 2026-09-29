use std::fs;
use std::path::PathBuf;

use ownai_core::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use ownai_language_elm::ElmProjector;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/elm");

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("fixture path is valid")).expect("supported path")
}

fn fixture_cases() -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(FIXTURES)
        .expect("the Elm fixture directory must exist")
        .map(|entry| entry.expect("fixture directory entry").path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    cases
}

#[test]
fn every_fixture_case_contains_its_inputs_and_expectations() {
    let cases = fixture_cases();
    assert!(
        !cases.is_empty(),
        "at least one Elm fixture case is required"
    );

    for case in cases {
        for file in ["input.elm", "types.txt", "signatures.txt"] {
            assert!(
                case.join(file).is_file(),
                "fixture {case:?} is missing {file}"
            );
        }
    }
}

#[test]
fn fixtures_project_to_their_expected_canonical_text() {
    for case in fixture_cases() {
        let source = fs::read_to_string(case.join("input.elm")).expect("read input.elm");
        let path = supported("input.elm");
        let projector = ElmProjector::new();

        for (mode, expectation) in [
            (ProjectionMode::Types, "types.txt"),
            (ProjectionMode::Signatures, "signatures.txt"),
        ] {
            let expected = fs::read_to_string(case.join(expectation)).expect("read expectation");
            let projected = projector
                .project(ProjectionInput {
                    path: &path,
                    source: &source,
                    mode,
                })
                .unwrap_or_else(|error| panic!("{case:?} {mode:?} failed to project: {error}"));

            assert_eq!(
                projected.canonical_text(),
                expected,
                "\nfixture {case:?} {mode:?} projection differs from {}",
                expectation
            );
        }
    }
}
