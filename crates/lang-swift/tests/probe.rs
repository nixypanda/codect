//! Gate A discovery probe: representative modern Swift syntax, including
//! executable bodies. Prints one line per snippet so findings are reviewable.
//!
//! Two important distinctions for the reviewer's findings:
//!   * `_read { ... }` alone is ACCEPTED by the parser (a clean misparse) — it is
//!     rejected by the narrow accessor-position check, not by `has_error()`.
//!   * malformed `#if`/`#else` sequences are also ACCEPTED by the parser — they
//!     are rejected by `plan_conditionals`' structural validation layer.

use std::fs;
use std::path::PathBuf;

use lang_swift::{
    CondError, CondRole, DirectiveKind, ParseReport, PlanError, plan_conditionals,
    read_in_accessor_position,
};

fn snippets() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "struct_stored_props",
            r##"
struct Point {
    var x: Double
    var y: Double = 0
    static let origin = Point(x: 0, y: 0)
}
"##,
        ),
        (
            "class_computed_observers",
            r##"
final class Counter: Base, Protocol {
    private(set) var value: Int = 0 {
        willSet { print(newValue) }
        didSet { print(oldValue) }
    }
    var doubled: Int { get { value * 2 } set { value = newValue / 2 } }
    override init() { super.init() }
    deinit { print("bye") }
}
"##,
        ),
        (
            "enum_assoc_raw",
            r##"
enum Result<T, E: Error>: CustomStringConvertible {
    case success(T)
    case failure(label: String, E)
    case blank
    var description: String { "..." }
}
"##,
        ),
        (
            "actor_isolation",
            r##"
actor Store<T: Sendable> {
    private var items: [T] = []
    func add(_ item: T) async { items.append(item) }
    nonisolated func describe() -> String { "store" }
}
"##,
        ),
        (
            "protocol_associatedtype",
            r##"
protocol Repository: AnyObject, Sendable {
    associatedtype Item: Identifiable where Item.ID == Int
    var all: [Item] { get async throws }
    func find(id: Int) -> Item?
    init(seed: Int)
}
"##,
        ),
        (
            "extension_conformance",
            r##"
extension Array: Hello where Element: Numeric {
    func greet() -> String { "hi" }
}
extension Int { static let magic = 42 }
"##,
        ),
        (
            "typealias_and_globals",
            r##"
public typealias Handler<T> = (T) -> Void
let globalConst: Int = 1
var globalVar: String
"##,
        ),
        (
            "func_effects_defaults",
            r##"
func fetch<T: Decodable>(
    _ url: URL,
    as type: T.Type = T.self,
    retries: Int = 3
) async throws -> T {
    fatalError()
}
"##,
        ),
        (
            "subscript_init_deinit",
            r##"
struct Matrix {
    subscript(row: Int, col: Int) -> Double {
        get { 0 }
        set { }
    }
    init(_ v: [Double]) { }
    deinit { }
}
"##,
        ),
        (
            "operator_precedence",
            r##"
infix operator ??? : AdditionPrecedence
precedencegroup AdditionPrecedence {
    associativity: left
    higherThan: MultiplicationPrecedence
}
prefix operator ^^
"##,
        ),
        (
            "nested_types",
            r##"
struct Outer {
    struct Inner { let x: Int }
    enum Kind { case a, b }
}
"##,
        ),
        (
            "swiftui_result_builder",
            r##"
import SwiftUI
struct ContentView: View {
    @State private var count = 0
    @ViewBuilder var body: some View {
        VStack {
            Text("Hello")
            if count > 0 {
                Text("\(count)")
            } else {
                EmptyView()
            }
        }
        .padding()
    }
}
"##,
        ),
        (
            "macros",
            r##"
import Observation
@Observable final class Model {
    var name: String = ""
}
@Test func simple() { #expect(1 == 1) }
#Preview { ContentView() }
@attached(member, names: named(init)) macro Member() = #externalMacro(module: "M", type: "T")
macro M(_ x: Int) -> Int
"##,
        ),
        (
            "property_wrappers",
            r##"
class C {
    @Published var text: String = ""
    @Environment(\.colorScheme) var scheme
    @State private var flag = false
}
"##,
        ),
        (
            "concurrency_modern",
            r##"
@MainActor func ui() async { await Task.yield() }
nonisolated(unsafe) let shared: Int = 0
func run() async {
    async let a = work()
    let tasks = [Task { await work() }]
    _ = await a
}
@Sendable func work() async -> Int { 1 }
"##,
        ),
        (
            "typed_throws",
            r##"
enum MyError: Error { case bad }
func f() throws(MyError) -> Int { throw .bad }
func g() rethrows -> Int { 0 }
let fn: () throws(MyError) -> Void = {}
"##,
        ),
        (
            "ownership_generics",
            r##"
struct Box<T: ~Copyable>: ~Copyable {
    borrowing func read() -> Int { 0 }
    consuming func take() -> Int { 0 }
    mutating func change(_ x: inout Int) { }
}
"##,
        ),
        (
            "parameter_packs",
            r##"
func zipAll<each T>(_ values: repeat each T) -> (repeat each T) {
    (repeat each values)
}
struct Tuple<each T> { var values: (repeat each T) }
"##,
        ),
        (
            "regex_and_strings",
            r##"
let r = /hello\s+world/
let raw = #"raw \n string"#
let multi = """
line one
line "two"
"""
let interpolated = "value: \(r) \(multi.count)"
"##,
        ),
        (
            "conditional_compilation",
            r##"
#if canImport(UIKit)
import UIKit
typealias PlatformView = UIView
#elseif os(macOS)
import AppKit
typealias PlatformView = NSView
#else
#error("unsupported")
#endif
"##,
        ),
        (
            "accessors_yielding",
            r##"
struct S {
    var storage: Int = 0
    var value: Int {
        get { storage }
        set { storage = newValue }
        _read { yield storage }
        _modify { yield &storage }
    }
}
"##,
        ),
        (
            "read_alone_clean_misparse",
            r##"
struct S {
    var v: Int {
        _read { yield storage }
    }
}
"##,
        ),
        (
            "read_call_in_getter_body",
            r##"
struct S {
    var v: Int {
        get { _read(1) }
    }
}
func _read(_ x: Int) -> Int { x }
"##,
        ),
        (
            "macro_braced_definition",
            r##"
macro M() { 1 }
"##,
        ),
        (
            "accessor_effects",
            r##"
protocol P {
    var a: Int { get async throws }
    var b: Int { get throws(MyError) }
}
struct S {
    var c: Int { mutating get { 0 } nonmutating set { } }
}
"##,
        ),
        (
            "nested_lambda_arguments",
            r##"
@foo(bar: outer({ inner({ leak() }) }))
struct S { func f(_ cb: () -> Void = outer({ inner({ leak() }) })) {} }
"##,
        ),
        (
            "interpolation_closure",
            r##"
let s = "values: \(items.map { print($0) }.joined()) end"
"##,
        ),
        (
            "opaque_existential_metatype",
            r##"
protocol P {}
func make() -> some P { X() }
var anyP: any P
let t: X.Type = X.self
let kp = \X.value
struct X: P { var value: Int }
"##,
        ),
        (
            "attribute_closures_defaults",
            r##"
func apply(_ f: () -> Void = { print("default") }) { f() }
@inline(__always) func fast() {}
"##,
        ),
        (
            "malformed_body",
            r##"
func broken() {
    let x = ; // invalid
}
"##,
        ),
        (
            "malformed_header",
            r##"
func broken( -> Int { 0 }
"##,
        ),
    ]
}

/// Snippets the raw grammar cannot parse cleanly. `_read`-alone and malformed
/// directives are deliberately NOT here: the parser accepts them and the adapter
/// must reject them with its own validation layer.
const KNOWN_UNPARSEABLE: &[&str] = &[
    "accessors_yielding",
    "macro_braced_definition",
    "malformed_body",
    "malformed_header",
];

fn report(name: &str) -> ParseReport {
    let (name, source) = snippets()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("unknown snippet {name}"));
    ParseReport::for_source(source).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn supported_modern_syntax_parses_cleanly() {
    for (name, source) in snippets() {
        if KNOWN_UNPARSEABLE.contains(&name) {
            continue;
        }
        let report = ParseReport::for_source(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            report.ok,
            "{name} unexpectedly failed: {:?}",
            report.error_nodes
        );
    }
}

#[test]
fn documented_grammar_limitations_are_fatal_parse_errors() {
    for (name, source) in snippets() {
        if !KNOWN_UNPARSEABLE.contains(&name) {
            continue;
        }
        let report = ParseReport::for_source(source).expect("grammar must still assign");
        assert!(!report.ok, "{name} unexpectedly parsed cleanly");
    }
}

#[test]
fn read_alone_is_a_clean_misparse_detected_narrowly() {
    // The parser ACCEPTS `_read { ... }` as a computed property with a bare
    // statements/call_expression child. Do not pretend has_error() catches it.
    let clean = report("read_alone_clean_misparse");
    assert!(
        clean.ok,
        "read-alone must parse cleanly: {:?}",
        clean.error_nodes
    );
    assert!(
        clean.sexp.contains("computed_property")
            && clean.sexp.contains("call_expression")
            && clean.sexp.contains("lambda_literal"),
        "expected the documented misparse shape: {}",
        clean.sexp
    );
    let (name, source) = snippets()
        .into_iter()
        .find(|(n, _)| *n == "read_alone_clean_misparse")
        .unwrap();
    let spans = read_in_accessor_position(source).expect("assignment");
    assert_eq!(spans.len(), 1, "one unsupported _read accessor in {name}");

    // A legitimate `_read(1)` call inside a real getter body must NOT be flagged.
    let (_, legit) = snippets()
        .into_iter()
        .find(|(n, _)| *n == "read_call_in_getter_body")
        .unwrap();
    assert!(
        read_in_accessor_position(legit)
            .expect("assignment")
            .is_empty()
    );
}

#[test]
fn combined_accessors_with_read_are_a_grammar_error() {
    let report = report("accessors_yielding");
    assert!(!report.ok);
}

#[test]
fn braced_macro_definition_is_invalid_unsupported_input() {
    // Not standard Swift macro implementation; the grammar rejects it. Document
    // as unsupported parser input rather than claiming it is a valid macro body.
    let report = report("macro_braced_definition");
    assert!(!report.ok, "braced macro definition must not parse cleanly");
    assert!(!report.error_nodes.is_empty());
}

#[test]
fn accessor_effects_are_visible_as_nodes_or_tokens() {
    let report = report("accessor_effects");
    assert!(report.ok);
    assert!(report.named_kinds.contains("throws"), "{}", report.sexp);
    assert!(
        report.named_kinds.contains("throws_clause"),
        "typed throws on accessor: {}",
        report.sexp
    );
    assert!(
        report.named_kinds.contains("mutation_modifier"),
        "mutating/nonmutating on accessors: {}",
        report.sexp
    );
    // `async` is an anonymous token; it must be captured token-wise by C.
    assert!(
        !report.named_kinds.contains("async"),
        "async must be anonymous, not a named node: {:?}",
        report.named_kinds
    );
}

#[test]
fn nested_lambdas_in_attributes_and_defaults_are_all_visible() {
    let report = report("nested_lambda_arguments");
    assert!(report.ok);
    assert_eq!(
        report.sexp.matches("lambda_literal").count(),
        4,
        "two nested lambdas in the attribute and two in the default arg: {}",
        report.sexp
    );
}

#[test]
fn interpolation_contains_executable_closure_nodes() {
    let report = report("interpolation_closure");
    assert!(report.ok);
    assert!(report.named_kinds.contains("interpolated_expression"));
    assert!(report.named_kinds.contains("lambda_literal"));
    assert!(
        report
            .sexp
            .contains("interpolated_expression value: (call_expression")
    );
}

const MALFORMED_DIRECTIVES: &[(&str, CondError)] = &[
    (
        "#else\nlet a = 1\n#endif\n",
        CondError::Unmatched(DirectiveKind::Else, (0, 5)),
    ),
    (
        "#if A\nlet a = 1\n#else\nlet b = 2\n#else\nlet c = 3\n#endif\n",
        CondError::DuplicateElse((0, 0)),
    ),
    (
        "#if A\nlet a = 1\n#else\nlet b = 2\n#elseif B\nlet c = 3\n#endif\n",
        CondError::ElseIfAfterElse((0, 0)),
    ),
    ("#if A\nlet a = 1\n", CondError::MissingEndIf((0, 0))),
];

#[test]
fn malformed_directives_are_accepted_by_grammar_but_rejected_by_planner() {
    for (source, expected) in MALFORMED_DIRECTIVES {
        let report = ParseReport::for_source(source).expect("assignment");
        assert!(
            report.ok,
            "grammar must accept malformed directives: {source:?}"
        );
        match plan_conditionals(source) {
            Err(PlanError::Validation(errors)) => {
                assert!(
                    errors
                        .iter()
                        .any(|e| discriminants(e) == discriminants(expected)),
                    "expected {expected:?}, got {errors:?} for {source:?}"
                );
            }
            other => panic!("expected validation error for {source:?}, got {other:?}"),
        }
    }
}

fn discriminants(error: &CondError) -> &'static str {
    match error {
        CondError::Unmatched(_, _) => "Unmatched",
        CondError::DuplicateElse(_) => "DuplicateElse",
        CondError::ElseIfAfterElse(_) => "ElseIfAfterElse",
        CondError::MissingEndIf(_) => "MissingEndIf",
    }
}

#[test]
fn conditional_plan_has_exact_keys_spans_and_parent_chains() {
    let source = "#if DEBUG\nstruct A {}\n#elseif RELEASE\n#else\nstruct B {}\n#endif\n";
    let plan = plan_conditionals(source).expect("valid conditional sequence");

    let group = plan
        .items
        .iter()
        .find(|i| i.role == CondRole::Group)
        .expect("group");
    assert_eq!(group.key, "FILE::cond::if:DEBUG");
    assert_eq!(group.parent_key, None);
    assert_eq!(group.header.as_deref(), Some("#if DEBUG"));
    let endif_end = source.find("#endif").expect("endif") + "#endif".len();
    assert_eq!(group.span, (0, endif_end));

    let branches: Vec<_> = plan
        .items
        .iter()
        .filter(|i| i.role == CondRole::Branch)
        .collect();
    assert_eq!(
        branches.len(),
        3,
        "all original branch headers are preserved"
    );
    assert_eq!(branches[0].key, "FILE::cond::if:DEBUG::branch::0::if:DEBUG");
    assert_eq!(
        branches[1].key,
        "FILE::cond::if:DEBUG::branch::1::elseif:RELEASE"
    );
    assert_eq!(branches[2].key, "FILE::cond::if:DEBUG::branch::2::else");
    for branch in &branches {
        assert_eq!(branch.parent_key.as_deref(), Some(group.key.as_str()));
    }
    assert_eq!(branches[1].header.as_deref(), Some("#elseif RELEASE"));
    // The empty `#elseif RELEASE` branch keeps its header and has no member.
    assert!(
        !plan
            .items
            .iter()
            .any(|i| i.parent_key.as_deref() == Some(branches[1].key.as_str()))
    );
    // Non-empty branches own their members.
    assert!(
        plan.items
            .iter()
            .any(|i| i.parent_key.as_deref() == Some(branches[0].key.as_str()))
    );
    assert!(
        plan.items
            .iter()
            .any(|i| i.parent_key.as_deref() == Some(branches[2].key.as_str()))
    );
}

#[test]
fn conditionals_are_recognized_in_every_declaration_container() {
    let cases = [
        (
            "struct S {\n#if D\nlet a: Int\n#else\nlet b: Int\n#endif\n}\n",
            "class_body",
        ),
        (
            "enum E {\n#if D\ncase a\n#else\ncase b\n#endif\n}\n",
            "enum_class_body",
        ),
        (
            "protocol P {\n#if D\nvar a: Int { get }\n#else\nfunc b()\n#endif\n}\n",
            "protocol_body",
        ),
    ];
    for (source, container) in cases {
        let plan = plan_conditionals(source).expect("valid conditional");
        assert!(
            plan.directives
                .iter()
                .any(|d| d.parent_container == container),
            "expected directives in {container}: {:?}",
            plan.directives
        );
        assert_eq!(plan.directives.len(), 3);
    }
}

/// Every golden fixture input must be grammar-clean on its own, independently of
/// whether `SwiftProjector` is implemented yet. This keeps a C failure on a
/// fixture distinguishable from an invalid fixture input.
#[test]
fn all_fixture_inputs_parse_cleanly() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/swift");
    let mut cases: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("read {}: {e}", root.display()))
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "expected Swift fixture cases");

    for case in cases {
        let input = case.join("input.swift");
        let source =
            fs::read_to_string(&input).unwrap_or_else(|e| panic!("read {}: {e}", input.display()));
        let name = case.file_name().expect("case name").to_string_lossy();
        let report = ParseReport::for_source(&source)
            .unwrap_or_else(|e| panic!("{name}: grammar assignment failed: {e}"));
        assert!(
            report.ok,
            "fixture {name} is not grammar-clean: {:?}\n{}",
            report.error_nodes, report.sexp
        );
    }
}

#[test]
fn print_probe_report() {
    for (name, source) in snippets() {
        match ParseReport::for_source(source) {
            Ok(report) => {
                println!(
                    "{name}: {} errs={:?}",
                    if report.ok { "OK " } else { "ERR" },
                    report.error_nodes
                );
            }
            Err(err) => println!("{name}: ASSIGN_ERROR {err}"),
        }
    }
}

#[test]
fn print_structural_sexps() {
    for (name, source) in snippets() {
        if !matches!(
            name,
            "enum_assoc_raw"
                | "conditional_compilation"
                | "macros"
                | "class_computed_observers"
                | "protocol_associatedtype"
                | "func_effects_defaults"
                | "swiftui_result_builder"
                | "accessor_effects"
                | "read_alone_clean_misparse"
        ) {
            continue;
        }
        if let Ok(report) = ParseReport::for_source(source) {
            println!("===== {name} =====\n{}", report.sexp);
        }
    }
}

#[test]
fn print_combined_inventory() {
    use std::collections::BTreeSet;
    let mut kinds = BTreeSet::new();
    let mut fields = BTreeSet::new();
    for (name, source) in snippets() {
        if let Ok(report) = ParseReport::for_source(source) {
            if !report.ok {
                println!("SKIP inventory for failing snippet: {name}");
                continue;
            }
            kinds.extend(report.named_kinds);
            fields.extend(report.fields);
        }
    }
    println!("ALL_NAMED_KINDS ({:?}):\n{:?}", kinds.len(), kinds);
    println!("ALL_FIELDS ({:?}):\n{:?}", fields.len(), fields);
}
