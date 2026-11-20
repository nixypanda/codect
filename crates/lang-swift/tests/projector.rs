use base::{
    ItemKind, LanguageProjector, ProjectionError, ProjectionInput, ProjectionMode, RepoPath,
    SupportedPath,
};
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

fn types(source: &str) -> String {
    text(source, ProjectionMode::Types)
}

fn signatures(source: &str) -> String {
    text(source, ProjectionMode::Signatures)
}

fn tests(source: &str) -> String {
    text(source, ProjectionMode::Tests)
}

// --- fatal errors and atomicity --------------------------------------------

#[test]
fn grammar_errors_are_fatal_even_inside_stripped_bodies() {
    let path = supported("src/lib.swift");
    for source in [
        "func broken( -> Int { 0 }\n",                // malformed header
        "func broken() {\n    let x = ; // bad\n}\n", // malformed body
        "macro M() { 1 }\n",                          // unsupported braced macro definition
    ] {
        let error = SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode: ProjectionMode::Signatures,
            })
            .expect_err("erroneous syntax must fail");
        assert!(
            matches!(error, ProjectionError::ErroneousSyntax { .. }),
            "{source:?} produced {error:?}"
        );
        assert!(error.range().is_some());
    }
}

#[test]
fn clean_misparsed_read_accessor_is_fatal_not_silently_accepted() {
    // The grammar accepts `_read { ... }` alone; contract §3/§5.1 require the
    // adapter to reject it in accessor position with its own validation layer,
    // reported as ErroneousSyntax at the offending accessor.
    let source = "struct S {\n    var v: Int {\n        _read { yield storage }\n    }\n}\n";
    let path = supported("src/lib.swift");
    let error = SwiftProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Signatures,
        })
        .expect_err("unsupported _read accessor must fail");
    assert!(
        matches!(error, ProjectionError::ErroneousSyntax { .. }),
        "{error:?}"
    );
    let range = error
        .range()
        .expect("ErroneousSyntax always carries a range");
    let read_at = source.find("_read").expect("accessor in source");
    assert_eq!(
        range.start_byte(),
        read_at,
        "the error must point at the offending _read accessor"
    );
    assert!(range.start_byte() < range.end_byte());
    let offending = &source[range.start_byte()..range.end_byte()];
    assert!(
        offending.trim_start().starts_with("_read"),
        "unexpected offending text {offending:?}"
    );
}

#[test]
fn combined_accessors_with_read_are_fatal() {
    let source = "struct S {\n    var storage: Int = 0\n    var value: Int {\n        get { storage }\n        set { storage = newValue }\n        _read { yield storage }\n        _modify { yield &storage }\n    }\n}\n";
    let path = supported("src/lib.swift");
    let error = SwiftProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Signatures,
        })
        .expect_err("combined accessors with _read must fail");
    assert!(
        matches!(error, ProjectionError::ErroneousSyntax { .. }),
        "{error:?}"
    );
    let range = error
        .range()
        .expect("ErroneousSyntax always carries a range");
    let read_at = source.find("_read").expect("accessor in source");
    assert!(
        range.start_byte() <= read_at && range.end_byte() > read_at,
        "the error range must cover the offending _read accessor: {range:?}"
    );
}

#[test]
fn malformed_conditional_directives_are_fatal_with_precise_spans() {
    // Every structural error from §9.3 is grammar-clean but must be rejected by
    // the adapter's supplemental validation as ErroneousSyntax. The nth
    // occurrence of `offending` identifies the exact directive at fault.
    let cases = [
        ("stray #else", "#else\nlet a = 1\n#endif\n", "#else", 0),
        ("stray #endif", "#endif\nlet a = 1\n", "#endif", 0),
        ("stray #elseif", "#elseif A\nlet a = 1\n", "#elseif", 0),
        (
            "duplicate #else",
            "#if A\nlet a = 1\n#else\nlet b = 2\n#else\nlet c = 3\n#endif\n",
            "#else",
            1,
        ),
        (
            "#elseif after #else",
            "#if A\nlet a = 1\n#else\nlet b = 2\n#elseif B\nlet c = 3\n#endif\n",
            "#elseif",
            0,
        ),
        ("missing #endif", "#if A\nlet a = 1\n", "#if", 0),
        (
            "nested missing #endif",
            "#if A\n#if B\nstruct X {}\n",
            "#if",
            0,
        ),
    ];
    let path = supported("src/lib.swift");
    for (name, source, offending, nth) in cases {
        let error = SwiftProjector::new()
            .project(ProjectionInput {
                path: &path,
                source,
                mode: ProjectionMode::Signatures,
            })
            .expect_err("malformed directive sequence must fail");
        assert!(
            matches!(error, ProjectionError::ErroneousSyntax { .. }),
            "{name}: {error:?}"
        );
        let range = error
            .range()
            .expect("ErroneousSyntax always carries a range");
        assert!(range.start_byte() < range.end_byte(), "{name}: empty span");
        let expected_at = source
            .match_indices(offending)
            .nth(nth)
            .unwrap_or_else(|| panic!("{name}: no occurrence {nth} of {offending:?}"))
            .0;
        assert_eq!(
            range.start_byte(),
            expected_at,
            "{name}: error must point at the offending directive"
        );
        let text = &source[range.start_byte()..range.end_byte()];
        assert!(
            text.starts_with(offending),
            "{name}: unexpected offending text {text:?}"
        );
    }
}

// --- mode differences and placeholders ------------------------------------

#[test]
fn protocol_keeps_typed_members_in_types_and_adds_callables_in_signatures() {
    let source = "protocol Repository: AnyObject, Sendable {\n    associatedtype Item: Identifiable where Item.ID == Int\n    var all: [Item] { get async throws }\n    func find(id: Int) -> Item?\n    init(seed: Int)\n}\n";
    assert_eq!(
        types(source),
        "protocol Repository: AnyObject, Sendable {\n    associatedtype Item: Identifiable where Item.ID == Int\n    var all: [Item] { get async throws }\n}\n"
    );
    assert_eq!(
        types(source),
        signatures(source).replace("    func find(id: Int) -> Item?\n    init(seed: Int)\n", "")
    );
}

#[test]
fn accessor_effects_are_retained_in_canonical_specifiers() {
    let source = "protocol P {\n    var a: Int { get async throws }\n    var b: Int { get throws(MyError) }\n}\nstruct S {\n    var c: Int { mutating get { 0 } nonmutating set { } }\n}\n";
    for out in [types(source), signatures(source)] {
        assert!(out.contains("var a: Int { get async throws }"), "{out}");
        assert!(out.contains("var b: Int { get throws(MyError) }"), "{out}");
        assert!(
            out.contains("var c: Int { mutating get nonmutating set }"),
            "{out}"
        );
    }
}

#[test]
fn enum_comma_cases_expand_and_raw_values_are_stripped() {
    let source = "enum E {\n    case a, b\n    case code = \"x\"\n}\n";
    let expected = "enum E {\n    case a\n    case b\n    case code\n}\n";
    assert_eq!(types(source), expected);
    assert_eq!(signatures(source), expected);
}

#[test]
fn observers_render_as_specifiers_only() {
    let source = "final class C {\n    var value: Int = 0 { willSet { print(newValue) } didSet { print(oldValue) } }\n}\n";
    assert_eq!(
        types(source),
        "final class C {\n    var value: Int { willSet didSet }\n}\n"
    );
}

#[test]
fn stored_properties_and_globals_drop_initializers() {
    let source = "struct S { var x: Int = 1 }\nlet g: Int = 2\n";
    let expected = "struct S {\n    var x: Int\n}\n\nlet g: Int\n";
    assert_eq!(signatures(source), expected);
    assert_eq!(types(source), "struct S {\n    var x: Int\n}\n");
}

#[test]
fn macro_definitions_are_stripped_and_never_expanded() {
    let source = "@attached(member, names: named(init)) macro Member() = #externalMacro(module: \"M\", type: \"T\")\nmacro M(_ x: Int) -> Int\n";
    let out = signatures(source);
    assert!(
        out.contains("@attached(member, names: named(init)) macro Member()"),
        "{out}"
    );
    assert!(out.contains("macro M(_ x: Int) -> Int"), "{out}");
    assert!(!out.contains("#externalMacro"), "{out}");
    assert_eq!(types(source), "");

    let file = project(source, ProjectionMode::Signatures);
    assert!(
        file.items()
            .iter()
            .any(|item| item.kind == ItemKind::Function && item.stable_key.contains("::macro::"))
    );
}

#[test]
fn missing_types_are_never_fabricated() {
    let source = "let a = 1\nvar b = \"\"\nstruct S { var c = 0 }\n";
    assert_eq!(types(source), "struct S {}\n");
    let out = signatures(source);
    assert!(out.contains("let a"), "{out}");
    assert!(out.contains("var b"), "{out}");
    assert!(out.contains("var c"), "{out}");
    assert!(!out.contains("= 1"), "{out}");
    assert!(!out.contains("= \"\""), "{out}");
    assert!(!out.contains("= 0"), "{out}");
    assert!(!out.contains(": Int"), "{out}");
    assert!(!out.contains(": String"), "{out}");
}

#[test]
fn tests_mode_declares_no_types() {
    // A @Test function is a test and must survive; an unrelated type is not a
    // test and must be filtered out.
    let source = "struct Unrelated { var x: Int }\n@Test func basic() {}\n";
    let out = tests(source);
    assert_eq!(out, "@Test func basic() { ... }\n", "{out}");
    assert!(!out.contains("struct Unrelated"), "{out}");
}

// --- modern syntax (grammar-validated) -------------------------------------

#[test]
fn modern_syntax_projects_without_leaking_bodies() {
    let cases = [
        (
            "actor",
            "actor Store<T: Sendable> {\n    private var items: [T] = []\n    func add(_ item: T) async { items.append(item) }\n    nonisolated func describe() -> String { \"store\" }\n}\n",
            &["actor Store<T: Sendable>", "func add(_ item: T) async"][..],
        ),
        (
            "typed throws",
            "enum MyError: Error { case bad }\nfunc f() throws(MyError) -> Int { throw .bad }\n",
            &["func f() throws(MyError) -> Int { ... }"][..],
        ),
        (
            "ownership",
            "struct Box<T: ~Copyable>: ~Copyable {\n    borrowing func read() -> Int { 0 }\n    consuming func take() -> Int { 0 }\n    mutating func change(_ x: inout Int) { }\n}\n",
            &[
                "borrowing func read() -> Int { ... }",
                "consuming func take() -> Int { ... }",
            ][..],
        ),
        (
            "parameter packs",
            "func zipAll<each T>(_ values: repeat each T) -> (repeat each T) {\n    (repeat each values)\n}\nstruct Tuple<each T> { var values: (repeat each T) }\n",
            &["func zipAll<each T>", "struct Tuple<each T>"][..],
        ),
        (
            "property wrappers",
            "class C {\n    @Published var text: String = \"\"\n    @Environment(\\.colorScheme) var scheme\n    @State private var flag = false\n}\n",
            &[
                "@Published var text: String",
                "@Environment(\\.colorScheme) var scheme",
                "@State private var flag",
            ][..],
        ),
        (
            "opaque and existential",
            "protocol P {}\nfunc make() -> some P { X() }\nvar anyP: any P\nlet t: X.Type = X.self\nstruct X: P { var value: Int }\n",
            &[
                "func make() -> some P { ... }",
                "var anyP: any P",
                "let t: X.Type",
            ][..],
        ),
    ];

    for (name, source, wanted) in cases {
        let out = signatures(source);
        for needle in wanted {
            assert!(out.contains(needle), "{name}: missing {needle:?} in {out}");
        }
        for leak in [
            "items.append",
            "fatalError",
            "print(",
            "yield ",
            "throw .bad",
            "repeat each values",
        ] {
            assert!(!out.contains(leak), "{name}: body leaked {leak:?} in {out}");
        }
    }
}

#[test]
fn unicode_and_escaped_identifiers_are_preserved() {
    let unicode = signatures("struct Café { var naïve: Int }\n");
    assert!(unicode.contains("Café"), "{unicode}");
    assert!(unicode.contains("naïve"), "{unicode}");

    let escaped = signatures("struct `class` { var `func`: Int }\n");
    assert!(escaped.contains("class"), "{escaped}");
    assert!(escaped.contains("func"), "{escaped}");
}

#[test]
fn nested_types_frame_their_members() {
    let source =
        "struct Outer {\n    struct Inner { let x: Int }\n    enum Kind { case a, b }\n}\n";
    let out = signatures(source);
    assert!(out.contains("struct Outer {"), "{out}");
    assert!(out.contains("struct Inner {"), "{out}");
    assert!(out.contains("enum Kind {"), "{out}");
    assert!(out.contains("case a"), "{out}");
    assert!(out.contains("case b"), "{out}");
}

// --- Tests mode ------------------------------------------------------------

#[test]
fn swift_testing_tests_are_detected_by_final_path_segment() {
    let source = "@Test func basic() { #expect(true) }\n@Test(arguments: [1, 2]) func parameterized(value: Int) { #expect(value > 0) }\n@Testing.Test func qualified() { }\nfunc notATest() { }\n";
    assert_eq!(
        tests(source),
        "@Test func basic() { ... }\n\n@Test(arguments: [1, 2]) func parameterized(value: Int) { ... }\n\n@Testing.Test func qualified() { ... }\n"
    );
}

#[test]
fn suite_alone_does_not_make_members_tests() {
    let source = "@Suite struct OnlySuite {\n    func helper() {}\n}\n";
    assert_eq!(tests(source), "");
}

#[test]
fn xctest_golden_from_contract_section_11() {
    let source = "@Test func basic() {\n    #expect(1 == 1)\n}\n\n@Suite struct MathTests {\n    @Test func add(a: Int, b: Int) {\n        #expect(a + b == 3)\n    }\n}\n\nfinal class FooTests: XCTestCase {\n    func testThing() {\n        XCTAssertTrue(true)\n    }\n    func helper() {}\n}\n\nextension FooTests {\n    func testExtension() {\n        XCTAssertTrue(true)\n    }\n}\n";
    assert_eq!(
        tests(source),
        "@Test func basic() { ... }\n\n@Suite struct MathTests {\n    @Test func add(a: Int, b: Int) { ... }\n}\n\nfinal class FooTests: XCTestCase {\n    func testThing() { ... }\n}\n\nextension FooTests {\n    func testExtension() { ... }\n}\n"
    );
}

#[test]
fn xctest_requires_direct_base_and_name_prefix_instance_zero_parameters() {
    let source = "final class Good: XCTest.XCTestCase {\n    func testOK() {}\n    static func testStatic() {}\n    func testParam(x: Int) {}\n    func test() {}\n    func TestUpper() {}\n}\nclass Custom: MyBase {\n    func testCustom() {}\n}\n";
    assert_eq!(
        tests(source),
        "final class Good: XCTest.XCTestCase {\n    func testOK() { ... }\n}\n"
    );
}

#[test]
fn xctest_ignores_the_return_type() {
    let source = "final class T: XCTestCase {\n    func testValue() -> Int { 1 }\n}\n";
    assert_eq!(
        tests(source),
        "final class T: XCTestCase {\n    func testValue() -> Int { ... }\n}\n"
    );
}

#[test]
fn xctest_extension_of_a_class_declared_elsewhere_is_excluded() {
    let source = "extension OtherTests {\n    func testX() {}\n}\n";
    assert_eq!(tests(source), "");
}
