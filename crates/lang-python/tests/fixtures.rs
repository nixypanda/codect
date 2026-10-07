// Expected projections are plain text so a failure shows a readable diff.

use std::fs;
use std::path::PathBuf;

use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_python::PythonProjector;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/python")
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

fn input_file(case: &std::path::Path) -> &'static str {
    if case.join("input.py").exists() {
        "input.py"
    } else {
        "input.pyi"
    }
}

#[test]
fn python_fixtures_match_expected_projections() {
    let projector = PythonProjector;
    let cases = fixture_cases();
    assert!(
        !cases.is_empty(),
        "expected at least one Python fixture case"
    );

    for case in cases {
        let name = case.file_name().expect("case name").to_string_lossy();
        let input_name = input_file(&case);
        let input = case.join(input_name);
        let source = fs::read_to_string(&input)
            .unwrap_or_else(|error| panic!("read {}: {error}", input.display()));
        let relative = format!("fixtures/python/{name}/{input_name}");
        let path = SupportedPath::new(RepoPath::new(relative.as_bytes()).expect("fixture path"))
            .expect("fixture path has a supported extension");

        let mut expectations = vec![
            (ProjectionMode::Types, "types.txt"),
            (ProjectionMode::Signatures, "signatures.txt"),
        ];
        // A case commits to Tests mode by shipping a `tests.txt`, and that file must be
        // non-empty: a broken detector also yields an empty string.
        let tests_expected = case.join("tests.txt");
        if tests_expected.exists() {
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
