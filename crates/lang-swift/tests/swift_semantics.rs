//! Independent regression tests for Swift declaration semantics that are
//! easy to get wrong: `_read` disambiguation, enum cases, operators,
//! subscripts, and backtick-escaped identifiers. Expectations come from the
//! written Swift source, not implementation output.

use base::{
    ItemKind, LanguageProjector, ProjectedFile, ProjectedItem, ProjectionError, ProjectionInput,
    ProjectionMode, RepoPath, SupportedPath,
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

fn tests(source: &str) -> String {
    text(source, ProjectionMode::Tests)
}

fn item<'a>(file: &'a ProjectedFile, suffix: &str) -> &'a ProjectedItem {
    let found: Vec<_> = file
        .items()
        .iter()
        .filter(|candidate| candidate.stable_key.ends_with(suffix))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected one item ending {suffix:?}: {found:?}"
    );
    found[0]
}

fn exact_item<'a>(file: &'a ProjectedFile, key: &str) -> &'a ProjectedItem {
    file.items()
        .iter()
        .find(|candidate| candidate.stable_key == key)
        .unwrap_or_else(|| panic!("missing item with exact key {key:?}"))
}

// --- _read: zero-arg call in shorthand getter is not an accessor ------------

#[test]
fn zero_arg_read_call_in_a_shorthand_getter_is_permitted() {
    // `tree-sitter-swift` accepts this (checked independently); only the braced
    // `_read { yield ... }` form is the unsupported accessor.
    let source = "func _read() -> Int { 0 }\nstruct S {\n    var x: Int { _read() }\n}\n";
    assert_eq!(
        signatures(source),
        "func _read() -> Int { ... }\n\nstruct S {\n    var x: Int { get }\n}\n"
    );
    let projected = file(source, ProjectionMode::Signatures);
    assert_eq!(
        item(&projected, "::function::_read()").kind,
        ItemKind::Function
    );
    assert_eq!(
        item(&projected, "::field::x").canonical_text,
        "    var x: Int { get }"
    );
}

#[test]
fn braced_read_accessor_is_unsupported_but_a_read_call_with_argument_is_not() {
    let bad = "struct S {\n    var x: Int { _read { yield storage } }\n}\n";
    let error = SwiftProjector::new()
        .project(ProjectionInput {
            path: &supported(),
            source: bad,
            mode: ProjectionMode::Signatures,
        })
        .expect_err("a braced _read accessor must be fatal");
    assert!(
        matches!(error, ProjectionError::ErroneousSyntax { .. }),
        "{error:?}"
    );

    let good =
        "func _read(_ x: Int) -> Int { x }\nstruct S {\n    var y: Int { get { _read(1) } }\n}\n";
    let projected = file(good, ProjectionMode::Signatures);
    assert_eq!(
        item(&projected, "::field::y").canonical_text,
        "    var y: Int { get }"
    );
}

// --- enum cases -------------------------------------------------------------

#[test]
fn heterogeneous_comma_cases_get_distinct_identity_and_exact_text() {
    let source = "enum E {\n    case a(Int), b(String), c\n}\n";
    let expected = "enum E {\n    case a(Int)\n    case b(String)\n    case c\n}\n";
    assert_eq!(types(source), expected);
    assert_eq!(signatures(source), expected);

    let projected = file(source, ProjectionMode::Signatures);
    for (suffix, name, canonical) in [
        ("::variant::a", "a", "    case a(Int)"),
        ("::variant::b", "b", "    case b(String)"),
        ("::variant::c", "c", "    case c"),
    ] {
        let found = item(&projected, suffix);
        assert_eq!(found.kind, ItemKind::Variant, "{suffix}");
        assert_eq!(found.name, name, "{suffix}");
        assert_eq!(found.canonical_text, canonical, "{suffix}");
    }
}

#[test]
fn an_indirect_case_modifier_is_retained() {
    let source = "enum Node {\n    indirect case next(Node)\n}\n";
    let expected = "enum Node {\n    indirect case next(Node)\n}\n";
    assert_eq!(types(source), expected);
    assert_eq!(signatures(source), expected);
    let projected = file(source, ProjectionMode::Signatures);
    let found = item(&projected, "::variant::next");
    assert_eq!(found.kind, ItemKind::Variant);
    assert_eq!(found.canonical_text, "    indirect case next(Node)");
}

// --- operators --------------------------------------------------------------

#[test]
fn postfix_operator_declaration_is_preserved() {
    let source = "postfix operator ++\n";
    assert_eq!(types(source), source);
    assert_eq!(signatures(source), source);
    let projected = file(source, ProjectionMode::Types);
    let found = item(&projected, "::operator::++");
    assert_eq!(found.kind, ItemKind::Operator);
    assert_eq!(found.canonical_text, "postfix operator ++");
}

// --- subscripts -------------------------------------------------------------

#[test]
fn subscript_get_set_effects_survive_body_only_edits() {
    let head = "protocol P {\n    subscript(row: Int, col: Int) -> Int { get set }\n}\n\n";
    let struct_a = "struct S {\n    subscript(row: Int, col: Int) -> Int {\n        get { row + col }\n        set { }\n    }\n}\n";
    let struct_b = "struct S {\n    subscript(row: Int, col: Int) -> Int {\n        get { row * col }\n        set { cache = newValue }\n    }\n}\n";
    let a = format!("{head}{struct_a}");
    let b = format!("{head}{struct_b}");

    assert_eq!(types(&a), types(&b), "body-only edit must not change Types");
    assert_eq!(
        signatures(&a),
        signatures(&b),
        "body-only edit must not change Signatures"
    );

    let projected = file(&a, ProjectionMode::Signatures);
    let protocol = exact_item(&projected, "src/lib.swift::trait::P");
    assert!(
        protocol.canonical_text.contains("get set"),
        "{}",
        protocol.canonical_text
    );
    let concrete = exact_item(
        &projected,
        "src/lib.swift::type::S::method::subscript(row:col:)",
    );
    assert_eq!(concrete.kind, ItemKind::Method);
    assert!(
        concrete.canonical_text.contains("get") && concrete.canonical_text.contains("set"),
        "{}",
        concrete.canonical_text
    );
    assert!(
        !signatures(&a).contains("row + col") && !signatures(&a).contains("cache = newValue"),
        "subscript body leaked: {}",
        signatures(&a)
    );
}

// --- backtick-escaped identifiers ------------------------------------------

#[test]
fn escaped_test_attribute_is_retained_on_a_signature() {
    let source = "@`Test` func t() {}\n";
    let out = signatures(source);
    assert!(out.contains("@`Test`"), "escaped attribute lost: {out}");
    assert!(out.contains("func t() { ... }"), "{out}");
}

#[test]
fn escaped_test_name_is_retained_in_tests_mode() {
    let source = "@Test func `testEscaped`() {}\n";
    assert_eq!(
        tests(source),
        "@Test func `testEscaped`() { ... }\n",
        "the backticks are part of the declared name"
    );
}

#[test]
fn escaped_xctest_class_name_is_retained_and_detected() {
    let source = "final class `FooTests`: XCTestCase {\n    func testThing() {}\n}\n";
    assert_eq!(
        tests(source),
        "final class `FooTests`: XCTestCase {\n    func testThing() { ... }\n}\n"
    );
}
