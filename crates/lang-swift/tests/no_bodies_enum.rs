//! Independent no-bodies regression tests for enum-case payload defaults and
//! case attributes. The expected text is the safe canonical form; it never
//! contains a token from a source body. Owned by packet D.

use base::{
    LanguageProjector, ProjectedFile, ProjectionInput, ProjectionMode, RepoPath, SupportedPath,
};
use lang_swift::SwiftProjector;

fn supported() -> SupportedPath {
    SupportedPath::new(RepoPath::new("src/lib.swift").unwrap()).unwrap()
}

fn file(source: &str, mode: ProjectionMode) -> ProjectedFile {
    SwiftProjector::new()
        .project(ProjectionInput {
            path: &supported(),
            source,
            mode,
        })
        .unwrap_or_else(|error| panic!("projection failed for {source:?}: {error}"))
}

fn text(source: &str, mode: ProjectionMode) -> String {
    file(source, mode).canonical_text().to_owned()
}

fn assert_no_body_leak(a: &str, b: &str, leaked: &[&str], expected: &str) {
    assert_eq!(
        text(a, ProjectionMode::Types),
        text(b, ProjectionMode::Types)
    );
    assert_eq!(
        text(a, ProjectionMode::Signatures),
        text(b, ProjectionMode::Signatures)
    );
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let out = text(a, mode);
        assert!(out.contains("{ ... }"), "placeholder missing: {out}");
        for token in leaked {
            assert!(!out.contains(token), "body token {token:?} leaked: {out}");
        }
        assert_eq!(out, expected, "unexpected canonical text in {mode:?}");
    }
}

#[test]
fn an_enum_case_payload_default_closure_body_does_not_leak() {
    let a = "enum E {\n    case cb(action: () -> Void = { secret() })\n}\n";
    let b = "enum E {\n    case cb(action: () -> Void = { otherBody() })\n}\n";
    assert_no_body_leak(
        a,
        b,
        &["secret", "otherBody"],
        "enum E {\n    case cb(action: () -> Void = { ... })\n}\n",
    );
    let projected = file(a, ProjectionMode::Signatures);
    assert_eq!(projected.items().len(), 2, "the enum and its one case");
    assert_eq!(
        projected.items()[1].stable_key,
        "src/lib.swift::type::E::variant::cb"
    );
}

#[test]
fn nested_calls_around_a_payload_default_closure_are_stripped_at_every_depth() {
    let a = "enum E {\n    case cb(action: () -> Void = outer({ inner({ leak() }) }))\n}\n";
    let b =
        "enum E {\n    case cb(action: () -> Void = outer({ inner({ differentBody() }) }))\n}\n";
    assert_no_body_leak(
        a,
        b,
        &["leak", "differentBody", "inner("],
        "enum E {\n    case cb(action: () -> Void = outer({ ... }))\n}\n",
    );
}

#[test]
fn an_enum_case_attribute_closure_body_does_not_leak() {
    let a = "enum E {\n    @foo(bar: { secret() })\n    case cb(Int)\n}\n";
    let b = "enum E {\n    @foo(bar: { otherBody() })\n    case cb(Int)\n}\n";
    assert_no_body_leak(
        a,
        b,
        &["secret", "otherBody"],
        "enum E {\n    @foo(bar: { ... }) case cb(Int)\n}\n",
    );
}
