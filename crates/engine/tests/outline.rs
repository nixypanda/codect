// Outline contract tests over the whole fixture corpus.
//
// The outline is derived from the Signatures projection, which must be a
// superset of the Types projection by `stable_key`. If any fixture violates
// that, the outline would silently omit a declaration the requested mode
// drops; this test fails rather than hiding it.

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use base::{Language, ProjectionMode, RepoPath};
use engine::project_source;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

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

fn keys(items: &[base::ProjectedItem]) -> Vec<String> {
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

#[test]
fn tests_mode_outline_is_the_signatures_superset_and_retains_only_tests() {
    let mut files = Vec::new();
    input_files(&fixture_root(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "expected at least one fixture input");

    // The corpus must contain both a retained test and a dropped declaration, or
    // both halves below pass vacuously.
    let mut saw_test = false;
    let mut saw_empty = false;

    for path in files {
        let display = path.to_string_lossy().into_owned();
        let language = repo_path_of(&path).language();

        // Elm and Haskell retain nothing, module header included, so an empty
        // projection means the same thing in every language.
        if !matches!(language, Some(Language::Rust) | Some(Language::Python)) {
            let tests = project_source(
                &repo_path_of(&path),
                &std::fs::read(&path).unwrap(),
                ProjectionMode::Tests,
            )
            .unwrap_or_else(|error| panic!("Tests projection of {display}: {error}"));
            assert!(
                tests.projection.items().is_empty(),
                "{display}: {} does not implement test detection, so its Tests \\
                 projection must retain nothing, including its module header",
                match language {
                    Some(Language::Elm) => "elm",
                    Some(Language::Haskell) => "haskell",
                    _ => "unsupported",
                }
            );
            continue;
        }

        let source = std::fs::read(&path).unwrap_or_else(|error| panic!("read {display}: {error}"));
        let repo_path = repo_path_of(&path);

        let tests = project_source(&repo_path, &source, ProjectionMode::Tests)
            .unwrap_or_else(|error| panic!("Tests projection of {display}: {error}"));
        let signatures = project_source(&repo_path, &source, ProjectionMode::Signatures)
            .unwrap_or_else(|error| panic!("Signatures projection of {display}: {error}"));

        let signature_keys: BTreeSet<&str> = signatures
            .projection
            .items()
            .iter()
            .map(|item| item.stable_key.as_str())
            .collect();

        // Every Tests declaration survives into Signatures, making it a valid superset.
        for item in tests.projection.items() {
            assert!(
                signature_keys.contains(item.stable_key.as_str()),
                "{display}: Tests item `{}` is missing from Signatures; \
                 the outline superset assumption is violated",
                item.stable_key
            );
        }

        // The outline enumerates the Signatures superset, unchanged by the mode.
        assert_eq!(
            tests
                .outline
                .iter()
                .map(|item| item.item.stable_key.clone())
                .collect::<Vec<_>>(),
            keys(signatures.projection.items()),
            "{display}: the Tests-mode outline must enumerate the Signatures superset"
        );

        let tests_keys: BTreeSet<&str> = tests
            .projection
            .items()
            .iter()
            .map(|item| item.stable_key.as_str())
            .collect();
        for item in &tests.outline {
            assert_eq!(
                tests.retained_in_mode(item),
                tests_keys.contains(item.item.stable_key.as_str()),
                "{display}: retained_in_mode disagrees with the Tests projection for `{}`",
                item.item.stable_key
            );
        }

        if tests
            .outline
            .iter()
            .any(|item| !tests.retained_in_mode(item))
        {
            saw_empty = true;
        }
        if !tests.projection.items().is_empty() {
            saw_test = true;
        }
    }

    assert!(
        saw_test,
        "the fixture corpus must contain a test declaration"
    );
    assert!(
        saw_empty,
        "the fixture corpus must contain a declaration Tests mode drops"
    );
}

#[test]
fn dropped_regions_never_swallow_a_retained_declaration() {
    let mut files = Vec::new();
    input_files(&fixture_root(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "expected at least one fixture input");

    let mut saw_dropped = false;
    for path in files {
        let display = path.to_string_lossy().into_owned();
        let source = std::fs::read(&path).unwrap_or_else(|error| panic!("read {display}: {error}"));
        let lines_text = std::str::from_utf8(&source)
            .unwrap_or_else(|error| panic!("{display}: fixture source is not UTF-8: {error}"));
        let lines: Vec<&str> = lines_text.lines().collect();
        let repo_path = repo_path_of(&path);

        for mode in [ProjectionMode::Types, ProjectionMode::Tests] {
            let file = project_source(&repo_path, &source, mode)
                .unwrap_or_else(|error| panic!("{mode:?} projection of {display}: {error}"));
            let by_key: std::collections::BTreeMap<&str, &base::OutlineItem> = file
                .outline
                .iter()
                .map(|item| (item.item.stable_key.as_str(), item))
                .collect();

            for item in &file.outline {
                if !file.retained_in_mode(item) {
                    continue;
                }
                let mut parent = item.item.parent_key.as_deref();
                while let Some(key) = parent {
                    let ancestor = by_key.get(key).unwrap_or_else(|| {
                        panic!(
                            "{display}: retained `{}` has missing ancestor `{key}`",
                            item.item.stable_key
                        )
                    });
                    assert!(
                        file.retained_in_mode(ancestor),
                        "{display} ({mode:?}): retained `{}` has dropped ancestor `{key}`",
                        item.item.stable_key
                    );
                    parent = ancestor.item.parent_key.as_deref();
                }
            }

            // The TUI line mask hides dropped lines and keeps retained
            // coverage. A retained region wholly inside a dropped span would
            // lose its body; Types only re-enables same-line retained starts,
            // declaration lines, and ends, and Tests derives visibility from
            // its historical rows.
            let regions = base::fold_regions(&file, &lines);
            for dropped in regions.iter().filter(|region| !region.retained) {
                saw_dropped = true;
                for retained in regions.iter().filter(|region| region.retained) {
                    assert!(
                        !(dropped.start_line < retained.start_line
                            && retained.start_line < dropped.end_line),
                        "{display} ({mode:?}): dropped region `{}` {}..{} strictly contains \
                         retained start `{}` at line {}; the TUI line mask would hide that \
                         retained region (only same-line start, declaration, and end lines are \
                         re-enabled for Types)",
                        dropped.stable_key,
                        dropped.start_line,
                        dropped.end_line,
                        retained.stable_key,
                        retained.start_line,
                    );
                }
            }
        }
    }

    assert!(
        saw_dropped,
        "the fixture corpus must contain declarations Types or Tests drop"
    );
}
