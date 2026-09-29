//! Outline contract tests over the whole fixture corpus.
//!
//! The outline is derived from the Signatures projection, which must be a
//! superset of the Types projection by `stable_key`. If any fixture violates
//! that, the outline would silently omit a declaration the requested mode
//! drops; this test fails rather than hiding it.

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ownai_core::{ProjectionMode, RepoPath};
use ownai_engine::project_source;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Collects every `input.*` file under `dir`, recursively.
fn input_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            input_files(&path, out);
        } else if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("input."))
        {
            out.push(path);
        }
    }
}

fn repo_path_of(path: &Path) -> RepoPath {
    let relative = path
        .strip_prefix(fixture_root())
        .expect("fixture is under the fixture root");
    let raw = relative.to_string_lossy().replace('\\', "/");
    support::repo_path(&raw)
}

fn keys(items: &[ownai_core::ProjectedItem]) -> Vec<String> {
    items.iter().map(|item| item.stable_key.clone()).collect()
}

#[test]
fn signatures_projection_is_a_superset_of_types_over_the_fixture_corpus() {
    let mut files = Vec::new();
    input_files(&fixture_root(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "expected at least one fixture input");

    // The corpus as a whole must contain at least one declaration Types drops;
    // otherwise the mode-independence assertions below would pass vacuously.
    let mut saw_dropped = false;

    for path in files {
        let display = path.to_string_lossy().into_owned();
        let source = std::fs::read(&path).unwrap_or_else(|error| panic!("read {display}: {error}"));
        let repo_path = repo_path_of(&path);

        let types = project_source(&repo_path, &source, ProjectionMode::Types)
            .unwrap_or_else(|error| panic!("Types projection of {display}: {error}"));
        let signatures = project_source(&repo_path, &source, ProjectionMode::Signatures)
            .unwrap_or_else(|error| panic!("Signatures projection of {display}: {error}"));

        let signature_keys: BTreeSet<&str> = signatures
            .projection
            .items()
            .iter()
            .map(|item| item.stable_key.as_str())
            .collect();

        // Every Types declaration survives into Signatures; otherwise the
        // outline superset assumption is violated.
        for item in types.projection.items() {
            assert!(
                signature_keys.contains(item.stable_key.as_str()),
                "{display}: Types item `{}` is missing from Signatures; \
                 the outline superset assumption is violated",
                item.stable_key
            );
        }

        // The Types-mode outline is the Signatures superset, not the Types
        // projection: it must enumerate exactly the Signatures projection, in
        // order, even though Types drops many declarations. This is the
        // property that makes the outline mode-independent, and it crosses two
        // genuinely different projections.
        assert_eq!(
            types
                .outline
                .iter()
                .map(|item| item.item.stable_key.clone())
                .collect::<Vec<_>>(),
            keys(signatures.projection.items()),
            "{display}: the Types-mode outline must enumerate the Signatures superset"
        );

        // In Signatures mode every outlined declaration is retained, because the
        // requested mode is the superset.
        for item in &signatures.outline {
            assert!(
                signatures.retained_in_mode(item),
                "{display}: Signatures mode must retain `{}`",
                item.item.stable_key
            );
        }

        // `retained_in_mode` in the Types outline is exactly membership in the
        // Types projection, so a dropped declaration is still located.
        let types_keys: BTreeSet<&str> = types
            .projection
            .items()
            .iter()
            .map(|item| item.stable_key.as_str())
            .collect();
        for item in &types.outline {
            assert_eq!(
                types.retained_in_mode(item),
                types_keys.contains(item.item.stable_key.as_str()),
                "{display}: retained_in_mode disagrees with the Types projection for `{}`",
                item.item.stable_key
            );
        }

        // The outline is not just a relabelled projection: the corpus must
        // contain declarations the requested mode drops, so these assertions
        // are not vacuous.
        if types
            .outline
            .iter()
            .any(|item| !types.retained_in_mode(item))
        {
            saw_dropped = true;
        }
    }

    assert!(
        saw_dropped,
        "the fixture corpus must contain a declaration Types mode drops"
    );
}
