use std::collections::HashSet;

use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_swift::SwiftProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("path")).expect("supported path")
}

fn file(source: &str, mode: ProjectionMode) -> base::ProjectedFile {
    let path = supported("src/lib.swift");
    SwiftProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .expect("projection")
}

fn keys(source: &str, mode: ProjectionMode) -> Vec<String> {
    file(source, mode)
        .items()
        .iter()
        .map(|item| item.stable_key.clone())
        .collect()
}

#[test]
fn every_stable_key_starts_with_the_repository_path() {
    let source = "struct S {\n    var x: Int\n    func f() {}\n}\n";
    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        for key in keys(source, mode) {
            assert!(
                key.starts_with("src/lib.swift::"),
                "unexpected key {key:?} in {mode:?}"
            );
        }
    }
}

#[test]
fn a_dropped_sibling_does_not_change_retained_keys() {
    // `func helper()` is dropped in Types but present in Signatures. Keys are
    // allocated in the Signatures superset before mode filtering, so the field's
    // key must be identical whether the callable sibling is present or not.
    let base = "struct S {\n    var typed: Int\n}\n";
    let with_dropped = "struct S {\n    var typed: Int\n    func helper() {}\n}\n";
    let base_types = keys(base, ProjectionMode::Types);
    let base_signatures = keys(base, ProjectionMode::Signatures);
    let dropped_types = keys(with_dropped, ProjectionMode::Types);
    let dropped_signatures = keys(with_dropped, ProjectionMode::Signatures);

    // Types must be byte-identical: the helper is dropped there.
    assert_eq!(base_types, dropped_types);

    // Signatures legitimately gains the helper key; every pre-existing key stays
    // exactly the same, and the helper is the *only* additional key.
    let base_set: HashSet<_> = base_signatures.iter().collect();
    let dropped_set: HashSet<_> = dropped_signatures.iter().collect();
    let added: Vec<_> = dropped_signatures
        .iter()
        .filter(|key| !base_set.contains(key))
        .collect();
    assert_eq!(
        added.len(),
        1,
        "unexpected key changes: {dropped_signatures:?}"
    );
    assert!(
        added[0].contains("helper"),
        "the only new key must be the helper: {:?}",
        added[0]
    );
    assert!(
        base_signatures.iter().all(|key| dropped_set.contains(key)),
        "no pre-existing Signatures key may change"
    );
}

#[test]
fn same_label_overloads_get_distinct_keys_with_ordinals() {
    let source = "func f(x: Int) {}\nfunc f(x: String) {}\n";
    let k = keys(source, ProjectionMode::Signatures);
    assert_eq!(k.len(), 2, "{k:?}");
    assert_ne!(k[0], k[1], "{k:?}");
    assert!(k.iter().any(|key| key.ends_with("~1")), "{k:?}");
}

#[test]
fn a_filtered_colliding_sibling_preserves_the_ordinal_in_tests() {
    // Both overloads normalize to the same callable identity `f(x:)`. Keys are
    // allocated once in the Signatures superset, so the @Test overload keeps the
    // `~1` ordinal even though the earlier colliding sibling is filtered out of
    // Tests mode.
    let source = "func f(x: Int) {}\n@Test func f(x: String) {}\n";
    let signatures = keys(source, ProjectionMode::Signatures);
    let tests = keys(source, ProjectionMode::Tests);

    assert_eq!(signatures.len(), 2, "{signatures:?}");
    assert_eq!(tests.len(), 1, "{tests:?}");

    let plain = signatures
        .iter()
        .find(|key| key.contains("f(x:)") && !key.contains('~'))
        .expect("first overload keeps the base key");
    let ordinal = signatures
        .iter()
        .find(|key| key.contains("f(x:)") && key.ends_with("~1"))
        .expect("second overload gets ~1");
    assert_ne!(plain, ordinal);
    assert_eq!(&tests[0], ordinal, "the surviving @Test key must keep ~1");
}

#[test]
fn different_label_overloads_get_distinct_keys_without_ordinals() {
    let source = "func f(x: Int) {}\nfunc f(y: Int) {}\n";
    let k = keys(source, ProjectionMode::Signatures);
    assert_eq!(k.len(), 2, "{k:?}");
    assert_ne!(k[0], k[1], "{k:?}");
    assert!(k.iter().all(|key| !key.contains('~')), "{k:?}");
}

#[test]
fn callable_keys_include_normalized_external_labels() {
    let source = "func subscribe(to: Int, for: Int) {}\n";
    let k = keys(source, ProjectionMode::Signatures);
    assert!(
        k.iter().any(|key| key.contains("subscribe(to:for:)")),
        "{k:?}"
    );
}

#[test]
fn keys_are_unchanged_by_body_and_initializer_edits() {
    let a = "func f(x: Int) -> Int { x }\nstruct S { var v: Int = 1 }\n";
    let b = "func f(x: Int) -> Int { x * 99 + helper() }\nstruct S { var v: Int = 42 + g() }\n";
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        assert_eq!(keys(a, mode), keys(b, mode), "{mode:?}");
    }
}

#[test]
fn types_and_tests_keys_are_a_subset_of_signatures() {
    let source = "struct S { var x: Int }\nfunc helper() {}\n@Test func t() {}\n";
    let signatures = keys(source, ProjectionMode::Signatures);
    for mode in [ProjectionMode::Types, ProjectionMode::Tests] {
        for key in keys(source, mode) {
            assert!(
                signatures.contains(&key),
                "{mode:?} key {key:?} missing from Signatures: {signatures:?}"
            );
        }
    }
}

#[test]
fn documented_kind_tokens_appear_in_keys() {
    let source = "struct S { var f: Int }\nlet g: Int = 1\ninfix operator ??? : AdditionPrecedence\ntypealias A = Int\nprotocol P { var p: Int { get } }\nfunc h() {}\nmacro M(_ x: Int) -> Int\n";
    let k = keys(source, ProjectionMode::Signatures);
    for token in [
        "::type::",
        "::field::",
        "::value::",
        "::operator::",
        "::type_alias::",
        "::trait::",
        "::function::",
        "::macro::",
    ] {
        assert!(
            k.iter().any(|key| key.contains(token)),
            "missing {token} in {k:?}"
        );
    }
}
