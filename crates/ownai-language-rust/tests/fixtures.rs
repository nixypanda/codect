//! Fixture-driven projection tests (TECHNICAL_DESIGN.md section 16.2).
//!
//! Expected projections are plain text so a failure shows a readable diff.

use std::fs;
use std::path::PathBuf;

use ownai_core::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use ownai_language_rust::RustProjector;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rust")
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
fn rust_fixtures_match_expected_projections() {
    let projector = RustProjector;
    let cases = fixture_cases();
    assert!(!cases.is_empty(), "expected at least one Rust fixture case");

    for case in cases {
        let name = case.file_name().expect("case name").to_string_lossy();
        let input = case.join("input.rs");
        let source = fs::read_to_string(&input)
            .unwrap_or_else(|error| panic!("read {}: {error}", input.display()));
        let relative = format!("fixtures/rust/{name}/input.rs");
        let path = SupportedPath::new(RepoPath::new(relative.as_bytes()).expect("fixture path"))
            .expect("fixture path has a supported extension");

        for (mode, expected_file) in [
            (ProjectionMode::Types, "types.txt"),
            (ProjectionMode::Signatures, "signatures.txt"),
        ] {
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
