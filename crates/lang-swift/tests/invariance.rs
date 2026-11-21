use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_swift::SwiftProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("path")).expect("supported path")
}

fn project(source: &str, mode: ProjectionMode) -> base::ProjectedFile {
    let path = supported("src/lib.swift");
    SwiftProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .expect("projection")
}

fn text(source: &str, mode: ProjectionMode) -> String {
    project(source, mode).canonical_text().to_owned()
}

fn both(source: &str) -> (String, String) {
    (
        text(source, ProjectionMode::Types),
        text(source, ProjectionMode::Signatures),
    )
}

fn tests(source: &str) -> String {
    text(source, ProjectionMode::Tests)
}

fn keys(source: &str, mode: ProjectionMode) -> Vec<String> {
    project(source, mode)
        .items()
        .iter()
        .map(|item| item.stable_key.clone())
        .collect()
}

// --- invariance: edits that must not change a projection -------------------

#[test]
fn changing_only_a_function_body_leaves_both_projections_unchanged() {
    let short = "func compute(a: Int) -> Int { a }\n";
    let long = "func compute(a: Int) -> Int {\n    let b = a * 2\n    return b + 1\n}\n";
    assert_eq!(both(short), both(long));
}

#[test]
fn changing_only_an_initializer_leaves_both_projections_unchanged() {
    let a = "struct S {\n    var x: Int = 1\n}\n";
    let b = "struct S {\n    var x: Int = 999 + complexCalculation()\n}\n";
    assert_eq!(both(a), both(b));
    assert!(!text(a, ProjectionMode::Types).contains('='));
}

#[test]
fn changing_comments_leaves_both_projections_unchanged() {
    let plain = "struct S {\n    var x: Int\n}\n";
    let commented =
        "// leading\n/// documented\nstruct S { /* inline */ var x: Int } // trailing\n";
    assert_eq!(both(plain), both(commented));
}

#[test]
fn changing_whitespace_leaves_both_projections_unchanged() {
    let spaced = "struct S {\n    var x: Int\n    var y: String\n}\n";
    let compact = "struct S{var x:Int\nvar y:String}\n";
    assert_eq!(both(spaced), both(compact));
}

#[test]
fn changing_an_observer_body_leaves_both_projections_unchanged() {
    let a = "final class C {\n    var value: Int = 0 { willSet { print(newValue) } didSet { print(oldValue) } }\n}\n";
    let b = "final class C {\n    var value: Int = 0 { willSet { log(\"new\") } didSet { log(\"old\") } }\n}\n";
    assert_eq!(both(a), both(b));
    let out = text(a, ProjectionMode::Types);
    assert!(out.contains("willSet didSet"), "{out}");
    assert!(!out.contains("print("), "{out}");
    assert!(!out.contains("newValue"), "{out}");
}

#[test]
fn changing_a_nested_closure_body_leaves_both_projections_unchanged() {
    let a = "@foo(bar: outer({ inner({ leakA() }) }))\nstruct S {}\n";
    let b = "@foo(bar: outer({ inner({ totallyDifferent(1, 2) }) }))\nstruct S {}\n";
    let (types, signatures) = both(a);
    assert_eq!(both(a), both(b));
    for out in [types, signatures] {
        assert!(!out.contains("leakA"), "{out}");
        assert!(!out.contains("totallyDifferent"), "{out}");
    }
}

#[test]
fn changing_an_interpolation_closure_body_leaves_both_projections_unchanged() {
    let a = "@foo(v: \"\\(items.map { print($0) }.joined())\")\nstruct S {}\n";
    let b = "@foo(v: \"\\(items.map { secret($0, 2) }.joined())\")\nstruct S {}\n";
    assert_eq!(both(a), both(b));
    let out = text(a, ProjectionMode::Types);
    assert!(
        out.contains("@foo(v: \"\\(items.map { ... }.joined())\")"),
        "{out}"
    );
    assert!(!out.contains("print("), "{out}");
    assert!(!out.contains("secret("), "{out}");
}

#[test]
fn changing_only_a_test_body_leaves_the_tests_projection_unchanged() {
    let a = "@Test func basic() { #expect(true) }\n";
    let b = "@Test func basic() {\n    let x = 1\n    #expect(x > 0)\n}\n";
    assert_eq!(tests(a), tests(b));
}

#[test]
fn adding_a_non_test_function_leaves_the_tests_projection_unchanged() {
    let base = "@Test func basic() {}\n";
    let changed = "@Test func basic() {}\nfunc helper() {}\n";
    assert_eq!(tests(base), tests(changed));
    assert_ne!(
        text(base, ProjectionMode::Signatures),
        text(changed, ProjectionMode::Signatures)
    );
}

// --- conditional guards are token-normalized ------------------------------

#[test]
fn conditional_guard_spacing_and_comments_do_not_change_the_projection() {
    let tight = "#if os(iOS)\nstruct A {}\n#endif\n";
    let spaced = "#if os ( iOS )\nstruct A {}\n#endif\n";
    let commented = "#if os(/*c*/iOS)\nstruct A {}\n#endif\n";
    assert_eq!(both(tight), both(spaced));
    assert_eq!(both(tight), both(commented));
    assert_eq!(
        keys(tight, ProjectionMode::Types),
        keys(spaced, ProjectionMode::Types)
    );
    assert_eq!(
        keys(tight, ProjectionMode::Signatures),
        keys(commented, ProjectionMode::Signatures)
    );
}

#[test]
fn changing_a_guard_condition_changes_both_projections() {
    let a = "#if A\nstruct S {}\n#endif\n";
    let b = "#if B\nstruct S {}\n#endif\n";
    let (at, asig) = both(a);
    let (bt, bsig) = both(b);
    assert_ne!(at, bt);
    assert_ne!(asig, bsig);
}

// --- attribute layout ------------------------------------------------------

#[test]
fn attribute_placement_newlines_are_irrelevant() {
    // Two or more attributes render one per line, before the declaration.
    let inline = "@foo @bar struct S {}\n";
    let lines = "@foo\n@bar\nstruct S {}\n";
    assert_eq!(both(inline), both(lines));
    assert_eq!(
        text(inline, ProjectionMode::Types),
        "@foo\n@bar\nstruct S {}\n"
    );

    // Exactly one attribute renders inline with the declaration when it fits.
    let one_line = "@foo\nstruct S {}\n";
    let one_inline = "@foo struct S {}\n";
    assert_eq!(both(one_line), both(one_inline));
    assert_eq!(text(one_line, ProjectionMode::Types), "@foo struct S {}\n");
}

#[test]
fn a_long_attribute_structurally_breaks_within_the_line_budget() {
    let source = "@foo(firstArgument: 1000000, secondArgument: 2000000, thirdArgument: 3000000, fourthArgument: 4000000)\nstruct S {}\n";
    let out = text(source, ProjectionMode::Types);
    assert!(out.lines().count() > 2, "long attribute must wrap: {out}");
    assert!(
        out.lines().all(|line| line.len() <= 80),
        "no canonical line may exceed LINE_WIDTH: {out}"
    );
}

// --- changes that must alter a projection ---------------------------------

#[test]
fn changing_a_type_changes_both_projections() {
    let a = "struct S {\n    var x: Int\n}\n";
    let b = "struct S {\n    var x: UInt64\n}\n";
    let (at, asig) = both(a);
    let (bt, bsig) = both(b);
    assert_ne!(at, bt);
    assert_ne!(asig, bsig);
}

#[test]
fn changing_a_function_signature_changes_only_signatures() {
    let a = "func f(a: Int) -> Int { a }\n";
    let b = "func f(a: Int) -> UInt { UInt(a) }\n";
    let (at, asig) = both(a);
    let (bt, bsig) = both(b);
    assert_eq!(at, bt);
    assert_ne!(asig, bsig);
}

#[test]
fn changing_a_modifier_changes_both_projections() {
    let a = "struct S {\n    var x: Int\n}\n";
    let b = "struct S {\n    static var x: Int\n}\n";
    let (at, asig) = both(a);
    let (bt, bsig) = both(b);
    assert_ne!(at, bt);
    assert_ne!(asig, bsig);
}

#[test]
fn changing_a_test_attribute_changes_the_tests_projection() {
    let plain = "@Test func t() {}\n";
    let parameterized = "@Test(arguments: [1, 2]) func t() {}\n";
    assert_ne!(tests(plain), tests(parameterized));
    assert!(
        tests(parameterized).contains("arguments:"),
        "{}",
        tests(parameterized)
    );
}

#[test]
fn a_type_change_is_invisible_to_tests_mode() {
    let a = "struct S { var x: Int }\n@Test func t() {}\n";
    let b = "struct S { var x: UInt }\n@Test func t() {}\n";
    assert_eq!(tests(a), tests(b));
    assert_ne!(
        text(a, ProjectionMode::Types),
        text(b, ProjectionMode::Types)
    );
}

#[test]
fn reordering_declarations_changes_projected_order() {
    let ab = "struct A {}\nstruct B {}\n";
    let ba = "struct B {}\nstruct A {}\n";
    let a = text(ab, ProjectionMode::Types);
    let b = text(ba, ProjectionMode::Types);
    assert_ne!(a, b);
    assert!(a.find("struct A").unwrap() < a.find("struct B").unwrap());
    assert!(b.find("struct B").unwrap() < b.find("struct A").unwrap());
}
