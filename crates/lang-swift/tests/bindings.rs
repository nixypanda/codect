//! Independent regression tests for Swift binding semantics and source spans.
//! Expectations are derived from the Swift source written below, not from
//! implementation output. Owned by packet D.

use base::{
    ItemKind, LanguageProjector, ProjectedFile, ProjectedItem, ProjectionInput, ProjectionMode,
    RepoPath, SupportedPath,
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

fn types(source: &str) -> String {
    text(source, ProjectionMode::Types)
}

fn signatures(source: &str) -> String {
    text(source, ProjectionMode::Signatures)
}

/// Locate exactly one item by its full stable-key suffix: no vague substring
/// matches and no ambiguity about which declaration is under test.
fn item<'a>(file: &'a ProjectedFile, suffix: &str) -> &'a ProjectedItem {
    let found: Vec<_> = file
        .items()
        .iter()
        .filter(|candidate| candidate.stable_key.ends_with(suffix))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one item ending {suffix:?}, got {found:?}"
    );
    found[0]
}

fn span_text<'a>(source: &'a str, item: &ProjectedItem) -> &'a str {
    &source[item.span.start_byte()..item.span.end_byte()]
}

// --- written annotations only: no cross-binding type propagation -----------

#[test]
fn mixed_annotated_globals_keep_only_their_written_types() {
    let source = "let a: Int = 1, b: String = \"x\", c = true\n";
    assert_eq!(types(source), "");
    assert_eq!(signatures(source), "let a: Int\n\nlet b: String\n\nlet c\n");

    let projected = file(source, ProjectionMode::Signatures);
    assert_eq!(projected.items().len(), 3);
    for (suffix, name, written) in [
        ("::value::a", "a", "let a: Int"),
        ("::value::b", "b", "let b: String"),
        ("::value::c", "c", "let c"),
    ] {
        let found = item(&projected, suffix);
        assert_eq!(found.kind, ItemKind::Value, "{suffix}");
        assert_eq!(found.name, name, "{suffix}");
        assert_eq!(found.canonical_text, written, "{suffix}");
    }
}

#[test]
fn an_untyped_binding_does_not_inherit_a_later_binding_annotation() {
    let source = "let a = 1, b: String = \"x\"\n";
    assert_eq!(signatures(source), "let a\n\nlet b: String\n");
    let projected = file(source, ProjectionMode::Signatures);
    assert_eq!(projected.items().len(), 2);
    assert_eq!(item(&projected, "::value::a").canonical_text, "let a");
    assert_eq!(
        item(&projected, "::value::b").canonical_text,
        "let b: String"
    );
}

#[test]
fn field_bindings_types_drops_only_the_untyped_binding() {
    let source = "struct S {\n    var x: Int = 1, y: String = \"s\", z = true\n}\n";
    assert_eq!(
        types(source),
        "struct S {\n    var x: Int\n    var y: String\n}\n"
    );
    assert_eq!(
        signatures(source),
        "struct S {\n    var x: Int\n    var y: String\n    var z\n}\n"
    );

    let in_types = file(source, ProjectionMode::Types);
    assert_eq!(item(&in_types, "::field::x").kind, ItemKind::Field);
    assert_eq!(item(&in_types, "::field::y").kind, ItemKind::Field);
    assert!(
        !in_types
            .items()
            .iter()
            .any(|i| i.stable_key.ends_with("::field::z")),
        "untyped field z must be absent from Types"
    );
    let in_signatures = file(source, ProjectionMode::Signatures);
    assert_eq!(
        item(&in_signatures, "::field::z").canonical_text,
        "    var z"
    );
}

#[test]
fn a_shared_trailing_annotation_types_every_binding_in_its_list() {
    let source = "var x, y: Int\n";
    assert_eq!(types(source), "");
    assert_eq!(signatures(source), "var x: Int\n\nvar y: Int\n");
    let projected = file(source, ProjectionMode::Signatures);
    assert_eq!(projected.items().len(), 2);
    assert_eq!(item(&projected, "::value::x").name, "x");
    assert_eq!(item(&projected, "::value::y").name, "y");
    assert_eq!(item(&projected, "::value::x").canonical_text, "var x: Int");
    assert_eq!(item(&projected, "::value::y").canonical_text, "var y: Int");
}

#[test]
fn an_annotation_does_not_cross_an_initialized_binding() {
    let globals = "var x: Int = 1, y\n";
    assert_eq!(signatures(globals), "var x: Int\n\nvar y\n");
    let projected = file(globals, ProjectionMode::Signatures);
    assert_eq!(projected.items().len(), 2);
    assert_eq!(item(&projected, "::value::y").canonical_text, "var y");

    let fields = "struct S {\n    var x: Int = 1, y\n}\n";
    assert_eq!(types(fields), "struct S {\n    var x: Int\n}\n");
    assert_eq!(
        signatures(fields),
        "struct S {\n    var x: Int\n    var y\n}\n"
    );
}

#[test]
fn a_tuple_pattern_is_one_binding_with_a_space_normalized_name() {
    let source = "let (a, b): (Int, String) = (1, \"x\")\n";
    assert_eq!(signatures(source), "let (a, b): (Int, String)\n");
    let projected = file(source, ProjectionMode::Signatures);
    assert_eq!(projected.items().len(), 1, "one pattern is one binding");
    let found = &projected.items()[0];
    assert_eq!(found.kind, ItemKind::Value);
    assert_eq!(found.name, "(a, b)");
    assert_eq!(found.canonical_text, "let (a, b): (Int, String)");
    assert!(found.stable_key.ends_with("::value::(a, b)"), "{found:?}");
}

// --- source spans: complete binding for folding, no leading attributes ------

#[test]
fn a_leading_attribute_is_excluded_from_the_binding_span() {
    let source = "@foo\nlet a: Int = 1\n";
    let projected = file(source, ProjectionMode::Signatures);
    let found = item(&projected, "::value::a");
    assert_eq!(span_text(source, found), "let a: Int = 1");
    assert!(!span_text(source, found).contains("@foo"));
    assert!(found.span.start_byte() > 0);
}

#[test]
fn an_initializer_is_inside_the_binding_span() {
    let source = "let a: Int = expensiveCalculation()\n";
    let projected = file(source, ProjectionMode::Signatures);
    let found = item(&projected, "::value::a");
    assert_eq!(
        span_text(source, found),
        "let a: Int = expensiveCalculation()"
    );
}

#[test]
fn computed_accessor_bodies_are_inside_the_binding_span() {
    let source = "struct S {\n    var x: Int {\n        get { value * 2 }\n        set { value = newValue }\n    }\n}\n";
    let projected = file(source, ProjectionMode::Signatures);
    let found = item(&projected, "::field::x");
    let covered = span_text(source, found);
    assert!(covered.contains("get { value * 2 }"), "{covered:?}");
    assert!(covered.contains("set { value = newValue }"), "{covered:?}");
}

#[test]
fn observer_bodies_are_inside_the_binding_span() {
    let source = "struct S {\n    var x: Int = 0 {\n        willSet { log(newValue) }\n        didSet { log(oldValue) }\n    }\n}\n";
    let projected = file(source, ProjectionMode::Types);
    let found = item(&projected, "::field::x");
    let covered = span_text(source, found);
    assert!(covered.contains("willSet { log(newValue) }"), "{covered:?}");
    assert!(covered.contains("didSet { log(oldValue) }"), "{covered:?}");
}

#[test]
fn a_later_binding_starts_at_its_own_declaration_not_the_first_keyword() {
    let source = "let a: Int = 1, b: String = \"x\"\n";
    let projected = file(source, ProjectionMode::Signatures);
    let first = item(&projected, "::value::a");
    let second = item(&projected, "::value::b");
    assert_ne!(first.span.start_byte(), second.span.start_byte());
    assert_eq!(span_text(source, first), "let a: Int = 1");
    assert_eq!(span_text(source, second), "b: String = \"x\"");
    let second_at = source.find("b: String").expect("second binding");
    assert_eq!(
        second.span.start_byte(),
        second_at,
        "the later binding must start at its own name"
    );
}

#[test]
fn a_test_body_is_inside_its_function_span() {
    let source = "@Test func t() {\n    #expect(true)\n}\n";
    let projected = file(source, ProjectionMode::Tests);
    assert_eq!(projected.items().len(), 1);
    let found = &projected.items()[0];
    assert!(span_text(source, found).contains("#expect(true)"));
}
