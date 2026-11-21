# Swift language support — frozen contract (implemented and verified)

Status: **Implemented and verified; behavior frozen.** This document was frozen
after the Gate A grammar/contract spike and reviewer fixes and has since been
implemented by packets B/C/D/E and validated by independent review and QA. It is
the durable behavior target, not an outstanding handoff; implementation did not
silently define product behavior. The §12 spike commands and results remain
historical Gate A evidence, not claims about the current adapter's test count or
crate count. No open approval blockers remain.

Generated on branch `feat/swift-language-support` against `main` at `c67909a`. Historical evidence is in §12; completion and review records are in
`docs/plans/swift-language-support.md`.

## 1. Scope of this gate

Done:

- Validated `tree-sitter-swift` 0.7.4 against Tree-sitter 0.27 (ABI and binding assignment).
- Probed modern Swift syntax **including executable bodies**, recording what parses, what misparses cleanly, and what the grammar accepts but is semantically invalid.
- Inventoried node kinds and field names (§4).
- Frozen item kinds, keys, conditional reconstruction/validation, placeholders, body/default/attribute/interpolation stripping, accessor effects, property rules, and Tests detection (§§5–11).
- Added reference spike algorithms for conditional reconstruction and narrow `_read` detection, with passing tests — historical Gate A evidence, recorded in §12, not a statement of the current adapter.

At Gate A these were still pending for later packets: full `extract.rs`/`render.rs`/`SwiftProjector`, `Language::Swift` registration, path detection, engine/CLI/schema/TUI wiring, `fixtures/swift/`, and user-facing doc propagation. The Gate A spike deliberately did not implement `LanguageProjector` and did not touch `base::Language`. This is a historical snapshot: B landed `Language::Swift`, `.swift` detection, JSON/schema/TUI wiring and a **skeleton** `SwiftProjector`; C then replaced that same definition with the production adapter. Neither the spike nor the skeleton was shipping Swift support.

## 2. Grammar dependency, ABI, and footprint

| Property | Finding | Evidence |
| --- | --- | --- |
| Crate | `tree-sitter-swift` 0.7.4 (latest at spike time) | `cargo search tree-sitter-swift` |
| License | MIT | crate `Cargo.toml` |
| Runtime deps | `tree-sitter-language` 0.1.x (workspace already resolves 0.1.8), `cc` (build) | crate `Cargo.toml` |
| Grammar ABI | `LANGUAGE_VERSION 15` in `src/parser.c` | `grep LANGUAGE_VERSION` → `15` |
| Tree-sitter ABI | Tree-sitter 0.27 supports ABI 13–15; `.into()` accepted | spike compiles; `set_language` succeeds |
| Rust binding | `LANGUAGE: LanguageFn = LanguageFn::from_raw(tree_sitter_swift)` | `bindings/rust/lib.rs` |
| Native toolchain | `cc` compiles `parser.c` + `scanner.c` (C11), external scanner (6 tokens). No Apple SDK, Swift compiler, SourceKit, dependency resolution, or macro expansion. | `build.rs`, `scanner.c` |
| Build cost | `parser.c` 22 MiB, `STATE_COUNT 11522`, `SYMBOL_COUNT 567`; `node-types.json` 678 KiB | `ls -lh src/*` |
| Lockfile delta | Adds only `tree-sitter-swift v0.7.4`; no other version changes | `git diff Cargo.lock` |

**Conclusion:** a genuine ABI-15 grammar with a modern `LanguageFn` binding, compatible with Tree-sitter 0.27 without regeneration or patching. Dependency metadata alone would not prove this (the crate's dev-dependency is Tree-sitter 0.23).

## 3. Parsing, validation, and failure policy

Parsing is all-or-nothing, extended with two validation layers the grammar does not provide:

1. Assign `tree_sitter_swift::LANGUAGE` to a fresh `Parser`; assignment failure → fatal `ProjectionError::AstInvariant`.
2. `parser.parse(source, None)`; `None` → fatal `ProjectionError::ParseFailed`.
3. Any ERROR/MISSING node anywhere (**including inside a stripped body**) → fatal `ProjectionError::ErroneousSyntax` at the first error range. No partial projection, no raw-source fallback, no error masking.
4. **Conditional validation:** reconstruct conditional sequences from flat directive siblings and validate stack integrity (§9). The grammar accepts malformed sequences without error nodes, so this is an additional fatal check, not a parse-error path.
5. **Narrow `_read` detection:** flag an unsupported `_read` accessor only when the exact identifier `_read` is in accessor position **and is followed by an accessor body** (`_read { ... }` / a trailing lambda). An ordinary zero-argument `_read()` call, a prefixed identifier such as `_reader`, and any use inside a real body are allowed; include cases the parser accepts cleanly (§5.1).
6. **Braced macro definitions** are invalid/unsupported input; the grammar already reports an ERROR node for them (§5.6).

## 4. Syntax-node / field inventory (evidence)

Across the probe corpus, the grammar produced **112 distinct named node kinds** and **30 distinct field names** (unchanged by the reviewer-fix probes). Full sets:

**Named kinds (112):**

```
array_literal, array_type, assignment, associatedtype_declaration, attribute,
await_expression, boolean_literal, call_expression, call_suffix, class_body,
class_declaration, comparison_expression, computed_getter, computed_property,
computed_setter, control_transfer_statement, custom_operator, deinit_declaration,
diagnostic, didset_clause, directive, directly_assignable_expression, else,
enum_class_body, enum_entry, enum_type_parameters, equality_constraint,
equality_expression, existential_type, external_macro_definition, function_body,
function_declaration, function_type, getter_specifier, identifier, if_statement,
import_declaration, inheritance_constraint, inheritance_modifier,
inheritance_specifier, init_declaration, integer_literal, interpolated_expression,
key_path_expression, lambda_literal, line_str_text, line_string_literal,
macro_declaration, macro_definition, macro_invocation, member_modifier, modifiers,
multi_line_str_text, multi_line_string_literal, multiplicative_expression,
mutation_modifier, navigation_expression, navigation_suffix, opaque_type,
operator_declaration, optional_type, parameter, parameter_modifier,
parameter_modifiers, pattern, precedence_group_attribute,
precedence_group_attributes, precedence_group_declaration, prefix_expression,
property_declaration, property_modifier, protocol_body, protocol_declaration,
protocol_function_declaration, protocol_property_declaration,
protocol_property_requirements, raw_str_end_part, raw_string_literal, regex_literal,
setter_specifier, simple_identifier, source_file, statements, subscript_declaration,
super_expression, suppressed_constraint, throw_keyword, throws, throws_clause,
tuple_expression, tuple_type, tuple_type_item, type_annotation, type_constraint,
type_constraints, type_identifier, type_pack_expansion, type_parameter,
type_parameter_pack, type_parameters, typealias_declaration, user_type,
value_argument, value_argument_label, value_arguments, value_binding_pattern,
value_pack_expansion, value_parameter_pack, visibility_modifier, where_keyword,
willset_clause, willset_didset_block
```

**Field names (30):**

```
body, bound_identifier, computed_value, condition, constrained_type,
data_contents, declaration_kind, default_value, definition, element, expr,
external_name, inherits_from, interpolation, lhs, mutability, name, op,
operation, operator, params, result, rhs, suffix, suppressed, target, text,
type, value, wrapped
```

**Declaration-family anchors:** `class_declaration` (`class_body`/`enum_class_body`) for struct/class/actor/enum/extension; `protocol_declaration` (`protocol_body`); `typealias_declaration`; `associatedtype_declaration`; `enum_entry`; `property_declaration`/`computed_property` with `willset_didset_block`; `protocol_property_declaration`; `function_declaration`/`protocol_function_declaration`; `init_declaration`/`deinit_declaration`/`subscript_declaration`; `operator_declaration`; `macro_declaration`/`macro_invocation`/`macro_definition`/`external_macro_definition`; `directive` (flat, §9); `parameter` with `default_value`; `throws`/`throws_clause`/`mutation_modifier`; type nodes (`user_type`, `array_type`, `optional_type`, `function_type`, `tuple_type`, `opaque_type`, `existential_type`, `type_pack_expansion`); `lambda_literal` (body hazard); `interpolated_expression` (interpolation hazard); `attribute`.

## 5. Grammar limitations and evidence (no unsupported syntax presented as working)

1. **`_read` accessors are not modeled, and the clean misparse must be detected narrowly.** The grammar has `computed_modify`/`modify_specifier` but no `computed_read`.
   - Alone: `_read { yield storage }` is **accepted** (`PARSE: OK`) as `computed_property → statements → call_expression(_read) → lambda_literal`. It must be rejected by the adapter, not assumed to fail via `has_error()`.
   - Combined with `get`/`set`/`_modify`: grammar ERROR nodes (`ERROR [66..130]`), fatal via rule 3.
   - A legitimate `_read(1)` call inside a real getter body is also accepted, nested under `computed_getter → statements`; it **must not** be flagged. An ordinary zero-argument `_read()` call in a shorthand getter is likewise **allowed** — an empty argument list is a call, not an accessor.
   - **Decision:** flag only when the exact identifier `_read` is in *accessor position* **and is followed by an accessor body** (a `{ ... }` / trailing lambda): e.g. a `statements` (or similar) child directly under `computed_property`/`protocol_property_requirements` whose first token is exactly `_read` and whose next token begins an accessor brace, or a bare `_read` token with a trailing lambda in that same direct-child set. Do **not** flag `_read()` (no trailing body), `_reader`/`_readSomething` (token-accurate, not a source-prefix test), strings/comments containing `_read`, or any use nested inside a recognized accessor node (`computed_getter`, `computed_setter`, `computed_modify`, `willset_clause`, `didset_clause`). Evidence: `read_alone_is_a_clean_misparse_detected_narrowly`.

2. **Conditional directives are flat and malformed sequences are grammar-valid.** `#if`/`#elseif`/`#else`/`#endif` appear as bare `directive` siblings in `source_file`, `class_body`, `enum_class_body`, and `protocol_body`, with no branch node or structured condition; `#error(...)` becomes `diagnostic`. The grammar **accepts** unmatched `#else`, duplicate `#else`, `#elseif` after `#else`, and missing `#endif` with no error nodes. **Decision:** token-aware reconstruction + explicit stack validation (§9), fatal on any structural error.

3. **`async` is an anonymous token.** `node-types.json` declares `async` as anon; S-expressions omit it. Effects on accessors and callables — `async`, `throws`, `rethrows`, typed `throws(E)`, and `mutating`/`nonmutating` — must be captured token-wise from source spans, not via named-child lookup. Evidence: `accessor_effects_are_visible_as_nodes_or_tokens` (`throws`, `throws_clause`, `mutation_modifier` are named; `async` is not).

4. **Closures in attributes/defaults occur at arbitrary depth.** A nested case yields four `lambda_literal` nodes (two nested in the attribute argument, two in the default argument). Every one must be replaced by the `{ ... }` placeholder token-wise at all depths (§7.4). Evidence: `nested_lambdas_in_attributes_and_defaults_are_all_visible`.

5. **String interpolation can contain executable closures.** `"\(items.map { print($0) }.joined())"` yields `line_string_literal → interpolated_expression → call_expression → lambda_literal`. Literal text is preserved; nested executable closure bodies inside interpolation are stripped (§7.5). Evidence: `interpolation_contains_executable_closure_nodes`.

6. **Braced macro definitions are invalid/unsupported parser input.** `macro M() { 1 }` produces an ERROR node (`ERROR [1..10]`) and is fatal. Standard Swift macros are implemented out-of-module via `#externalMacro(...)`; there is no braced macro implementation in ordinary Swift. Do not describe a braced body as a standard macro implementation. Evidence: `braced_macro_definition_is_invalid_unsupported_input`.

7. **No macro expansion, inference, visibility filtering, or cross-file resolution.** `#Preview`/`#expect` never synthesize declarations; untyped declarations render without a fabricated type; private and public declarations are both retained; XCTest bases are same-file only.

8. **`precedencegroup` bodies are metadata.** Omit the declaration; preserve precedence relationships by name in the operator line (`infix operator ??? : AdditionPrecedence`).

9. **Malformed syntax in stripped bodies is fatal** (evidence: `malformed_body` → `ERROR [27..29]`). This is correct behavior.

## 6. ItemKind mapping (no new kinds; active schemas widen only)

| Construct | ItemKind | Types | Signatures |
| --- | --- | --- | --- |
| `struct` / `class` / `actor` / `enum` (incl. nested) | `Type` | ✅ | ✅ |
| `protocol` | `Trait` | ✅ header, generics, inheritance, associated/nested types, **typed properties** | ✅ + callable requirements |
| `extension` **with a conformance** | `TraitImplementation` | ✅ header survives even when it has no type-level members | ✅ header + members |
| `extension` **without a conformance**, with type-level members | `TraitImplementation` | ✅ header + retained type-level members | ✅ header + members |
| non-conformance `extension` whose only members are callables (methods/functions/values) | `TraitImplementation` | omitted — callables are not type-level members, mirroring an empty inherent `impl` | ✅ header + retained callables |
| empty, non-conformance `extension` | — | omitted | omitted |
| `enum case` | `Variant` | ✅ | ✅ |
| `typealias` | `TypeAlias` | ✅ | ✅ |
| `associatedtype` | `AssociatedType` | ✅ | ✅ |
| typed stored/computed property, **including protocol typed properties** | `Field` | ✅ | ✅ |
| untyped property/requirement without written type | `Field` | omitted | ✅ (no fabricated type) |
| global `let` / `var` | `Value` | omitted | ✅ (written type only, no initializer) |
| free `func` | `Function` | omitted | ✅ |
| member/requirement `func` | `Method` | omitted | ✅ |
| `init` | `Constructor` | omitted | ✅ |
| `deinit` | `Method` (name `deinit`) | omitted | ✅ |
| `subscript` | `Method` (name `subscript`) | omitted | ✅ |
| `operator` declaration | `Operator` | ✅ | ✅ |
| `macro` declaration (`Function`, key token `macro`; definition stripped) | `Function` | omitted | ✅ |
| conditional GROUP / BRANCH (synthetic) | `Module` | per §9 retention | per §9 retention |
| `import`, comments, doc comments, locals, `precedencegroup`, macro invocations, `#error`/`#warning` | — | omitted | omitted |

**Extension retention (original mode semantics).** An extension is retained in a
mode iff it names a conformance or contains at least one member retained in that
mode. A conformance header therefore survives even with no visible members. A
non-conformance extension is retained in Types only for type-level members
(nested types, aliases, associated types, typed properties, enum cases,
operators); a method-only or value-only non-conformance extension is **omitted
from Types** but retained in Signatures for its callables, exactly as an inherent
`impl` with no type information is omitted in Rust. An empty non-conformance
extension is omitted in every mode. Do not infer that every callable-only
extension survives Types.

**Multi-binding declarations.** Each source binding pattern is one projected
item: global bindings are `Value`s (`let a = 1, b = 2` produces two items keyed
by `a` and `b`), while type-level property bindings are `Field`s. A tuple pattern
(`let (a, b) = pair`) is one item whose canonical pattern name is `(a, b)` so its
stable key is non-empty; its written tuple annotation stays on the tuple
pattern, the initializer is removed, and element types are never inferred. A
shared trailing annotation propagates **only** through preceding uninitialized
simple bindings of the same source declaration where it is syntactically shared
(`var x, y: Int` gives both `x` and `y` the written `Int`); it is never copied
onto an initialized untyped binding (`let c = 3` keeps only its name) and never
crosses an initialized binding. No type is inferred from an initializer. Each
item's source span covers the complete binding, including its stripped
implementation, not merely its type annotation.

No new `ItemKind` variant is introduced; GROUP/BRANCH reuse `Module`. **Schema handling:** active `codect.show.v1.json` and `codect.diff.v2.json` widen the `language` enum with `swift` and gain `Language::Swift`; archived `codect.diff.v1.json` is left unchanged. No new `schema` value, no version bump. Consumers must tolerate the widened enum (existing contract rule: enum widening is a compatibility change).

## 7. Canonical rendering, placeholders, and no-bodies guarantees

### 7.1 General
Deterministic, UTF-8, compiler/environment independent; one trailing newline; one blank line between top-level declarations. Modifiers are normalized on the declaration line in source order. Attributes are token-normalized in source order and never depend on source whitespace: a single attribute renders inline on the declaration line when it fits; two or more attributes render one per line immediately before the declaration, in source order (§8.7). Doc comments and ordinary comments omitted. Containers render `Header {` / members at +4 spaces / `}`; empty container `Header {}`.

### 7.2 Body placeholder (frozen per family)
| Family | Canonical form |
| --- | --- |
| concrete `func` / method / `init` / `deinit` | header + ` { ... }` |
| protocol `func` / `init` requirement | header only, no placeholder |
| concrete or protocol `subscript`, including a setter-only subscript | `subscript(i: Int) -> Int { get set }` — accessor specifiers only, **with retained mutability/effects** (§7.3), never accessor bodies; a concrete shorthand getter emits `{ get }`, a protocol preserves the written accessors |
| computed property / protocol property requirement | `var name: Type { get set }` — specifiers only, **with retained effects** (§7.3) |
| observer property | `var name: Type { willSet didSet }` |
| stored property / global value | no initializer, no `=` |

`{ ... }` is a projection placeholder, never executable source; it is not used
for subscripts, whose accessor interface is the retained content. An
accessor-position `_read { ... }` is unsupported and fatal (§5.1); an ordinary
`_read()` call is not.

### 7.3 Accessor effects
Accessor specifiers retain their written effects: `get`, `set`, `_modify`, `willSet`, `didSet`, plus `async`, `throws`, `throws(E)`/`throws_clause`, `rethrows`, `mutating`, `nonmutating`, and isolation modifiers. Because `async` and some modifiers are anonymous tokens, capture effects from the source span of the accessor node with a token-aware walk, not from named children. Canonical accessor order: `mutating`/`nonmutating` → `get`/`set`/`_modify`/`willSet`/`didSet` → `async` → `throws`/`rethrows`/`throws(E)`. Example: `var all: [Item] { get async throws }`; `var b: Int { get throws(MyError) }`; `var c: Int { mutating get nonmutating set }`.

### 7.4 Defaults, attributes, and nested closure stripping
Default arguments are part of the interface and are retained; stored-property initializers are not. Non-closure defaults render as normalized source text (`retries: Int = 3`, `type: T.Type = T.self`). **Every** `lambda_literal` anywhere inside an attribute argument or default expression — including nested inside other lambdas, calls, arrays, or conditionals — is replaced by `{ ... }`, token-wise at all depths. So `@foo(bar: outer({ inner({ leak() }) }))` renders `@foo(bar: outer({ ... }))`, and a default `outer({ inner({ leak() }) })` renders `outer({ ... })`. No executable statement from any depth may reach `canonical_text`.

### 7.5 String/regex literals and interpolation
Line, raw, multiline, and regex literals are normalized as written; comment-like text inside literals is never treated as a comment. Literal text (delimiters, fixed text, interpolation punctuation) is preserved. **Within** an `interpolated_expression`, nested executable closure bodies are replaced with `{ ... }` under the same no-bodies rule; the surrounding literal text is untouched. Example: `"values: \(items.map { print($0) }.joined())"` → `"values: \(items.map { ... }.joined())"`.

### 7.6 Wrapping (shared architecture, no adapter refactor)
Wrapping is produced only by `base::doc::Doc`, never by ad-hoc splitting:
- `crates/base/src/render.rs` defines `pub const LINE_WIDTH: usize = 80`.
- `crates/base/src/doc.rs` defines `Doc::{Text, Line, SoftLine, SoftNil, Broken, Reserve, Indent, Group, Concat}`, `render(&Doc, depth)`, and `flat_width`; `Group` renders flat iff `column + flat_width + reserved <= LINE_WIDTH`.
- Use the same shape as `lang-rust/src/render.rs::bracket_list`: `Doc::Group(Concat[Text(open), Indent(Concat[SoftNil, item, (",", SoftLine)…, Broken(",")]), SoftNil, Text(close)])`. `Broken(",")` emits a trailing comma only when broken; `SoftNil` emits a newline only when broken. Render at the item's depth so a nested fragment matches the text embedded in its ancestor.
- No changes to existing adapters; no replacement of the shared renderer.

## 8. Frozen golden examples

`{ ... }` is the placeholder. Text shown is the exact canonical output for each mode.

### 8.1 Struct
Source:
```swift
struct Point: Equatable {
    var x: Double
    var y: Double = 0
    static let origin: Point = Point(x: 0, y: 0)
    func shifted(by dx: Double) -> Point { Point(x: x + dx, y: y) }
}
```
Types:
```
struct Point: Equatable {
    var x: Double
    var y: Double
    static let origin: Point
}
```
Signatures:
```
struct Point: Equatable {
    var x: Double
    var y: Double
    static let origin: Point
    func shifted(by dx: Double) -> Point { ... }
}
```

### 8.2 Protocol — typed properties retained in Types (corrected)

Typed properties are type-level member shape and are kept in **both** modes, including protocol property requirements; only callable requirements are added in Signatures.

Source:
```swift
protocol Repository: AnyObject, Sendable {
    associatedtype Item: Identifiable where Item.ID == Int
    var all: [Item] { get async throws }
    func find(id: Int) -> Item?
    init(seed: Int)
}
```
Types:
```
protocol Repository: AnyObject, Sendable {
    associatedtype Item: Identifiable where Item.ID == Int
    var all: [Item] { get async throws }
}
```
Signatures:
```
protocol Repository: AnyObject, Sendable {
    associatedtype Item: Identifiable where Item.ID == Int
    var all: [Item] { get async throws }
    func find(id: Int) -> Item?
    init(seed: Int)
}
```

### 8.3 Enum
Source:
```swift
enum Result<T, E: Error> {
    case success(T)
    case failure(label: String, E)
    case blank
    case code = "x"
    var isOK: Bool { true }
}
```
Types and Signatures (identical):
```
enum Result<T, E: Error> {
    case success(T)
    case failure(label: String, E)
    case blank
    case code
    var isOK: Bool { get }
}
```
`case a, b` expands to two `Variant` items; raw-value initializers are stripped.

### 8.4 Class — observers, accessors, effects
Source:
```swift
final class Counter: Base {
    private(set) var value: Int = 0 { willSet { print(newValue) } didSet { print(oldValue) } }
    var doubled: Int { get { value * 2 } set { value = newValue / 2 } }
    var written: Int { mutating get { 0 } nonmutating set { } }
    deinit { print("bye") }
}
```
Types:
```
final class Counter: Base {
    private(set) var value: Int { willSet didSet }
    var doubled: Int { get set }
    var written: Int { mutating get nonmutating set }
}
```
Signatures adds `deinit { ... }`.

### 8.5 Extension
Source:
```swift
extension Array: Hello where Element: Numeric {
    func greet() -> String { "hi" }
}
extension Int {}
```
Types:
```
extension Array: Hello where Element: Numeric {}
```
Signatures:
```
extension Array: Hello where Element: Numeric {
    func greet() -> String { ... }
}
```
The conformance header survives in Types with no visible members; the method is
a Signatures-only member. Empty non-conformance `extension Int {}` is omitted in
every mode, and a non-conformance extension holding only methods is likewise
omitted from Types (retained in Signatures).

### 8.6 Globals, typealias, operator, macro (definition stripped)
Source:
```swift
typealias Handler<T> = (T) -> Void
let globalConst: Int = 1
var globalVar: String
infix operator ??? : AdditionPrecedence
@attached(member, names: named(init)) macro Member() = #externalMacro(module: "M", type: "T")
macro M(_ x: Int) -> Int
```
Types:
```
typealias Handler<T> = (T) -> Void

infix operator ??? : AdditionPrecedence
```
Signatures:
```
typealias Handler<T> = (T) -> Void

let globalConst: Int

var globalVar: String

infix operator ??? : AdditionPrecedence

@attached(member, names: named(init)) macro Member()

macro M(_ x: Int) -> Int
```
Independent top-level declarations are separated by one blank line (`base::model::derive_canonical_text`, §7.1); nested members and conditional branch children keep a single newline. `macro_definition`/`external_macro_definition` are stripped completely; no macro expansion. Macro keys use token `macro`. `precedencegroup` bodies are omitted.

### 8.7 Attribute/default closure stripping (one level and nested)
Source:
```swift
@foo(bar: { print("leak") })
@deep(bar: outer({ inner({ leak() }) }))
struct S {
    func f(_ cb: () -> Void = { print("leak") }, _ d: () -> Void = outer({ inner({ leak() }) })) {}
}
```
Types:
```
@foo(bar: { ... })
@deep(bar: outer({ ... }))
struct S {}
```
Signatures:
```
@foo(bar: { ... })
@deep(bar: outer({ ... }))
struct S {
    func f(_ cb: () -> Void = { ... }, _ d: () -> Void = outer({ ... })) { ... }
}
```

### 8.8 Interpolation closure stripping
Source:
```swift
let s: String = "values: \(items.map { print($0) }.joined()) end"
```
Signatures:
```
let s: String
```
The initializer is a global-value initializer and is removed entirely (no type is fabricated when a global has no written type). When the same literal appears **inside a retained signature element** (e.g. a default argument `@foo(v: "\(items.map { print($0) }.joined())")`), it renders `@foo(v: "\(items.map { ... }.joined())")`: literal text preserved, nested closure body replaced.

### 8.9 Conditional compilation — all headers preserved, empty branch kept
Source:
```swift
#if DEBUG
struct A {}
#elseif RELEASE
#else
struct B {}
#endif
```
Types and Signatures (the empty `#elseif RELEASE` branch keeps its header):
```
#if DEBUG
struct A {}
#elseif RELEASE
#else
struct B {}
#endif
```
Retention with a type in one branch and a callable in another:
```swift
#if DEBUG
struct A {}
#else
func f() {}
#endif
```
Types (group retained because branch 0 has a retained member; branch 1 becomes header-only):
```
#if DEBUG
struct A {}
#else
#endif
```
Signatures:
```
#if DEBUG
struct A {}
#else
func f() { ... }
#endif
```
If **no** branch has a retained member, the whole sequence is dropped — no empty standalone `#if/#endif`.

### 8.10 Long wrapping golden (via `base::doc`)
Source:
```swift
func configure<Value: Codable & Sendable>(
    _ identifier: String,
    using decoder: JSONDecoder = JSONDecoder(),
    forKey key: String = "default-key",
    withCompletion completion: @escaping (Result<Value, Error>) -> Void
) async throws -> Value {
    fatalError()
}
```
The flat parameter list exceeds 80 columns, so `Doc::Group` breaks it; `Broken(",")` adds the trailing comma:
```
func configure<Value: Codable & Sendable>(
    _ identifier: String,
    using decoder: JSONDecoder = JSONDecoder(),
    forKey key: String = "default-key",
    withCompletion completion: @escaping (Result<Value, Error>) -> Void,
) async throws -> Value { ... }
```
A short signature such as `func shifted(by dx: Double) -> Point { ... }` stays on one line. Nested items render with relative `Doc::Indent`, so a method inside a type wraps at the same 80-column budget with 8-space continuation.

## 9. Conditional compilation — frozen reconstruction contract

### 9.1 Why
The grammar flattens directives and accepts malformed sequences (§5.2). Guards are **never dropped**. The adapter reconstructs a conditional tree from the flat `directive` siblings using a token-aware stack in **every** declaration container: `source_file`, `class_body`, `enum_class_body`, `protocol_body` (evidence: `conditionals_are_recognized_in_every_declaration_container`).

### 9.2 Algorithm (reference in `crates/lang-swift/src/lib.rs::plan_conditionals`)
- Read each `directive` node's source text by its byte span; classify the first token as `if`/`elseif`/`else`/`endif`.
- Normalize the guard with a token-aware pass, not `split_whitespace`: drop comments and normalize punctuation so `os(iOS)` and `os ( iOS )` are the same guard. Use that normalized header for both keys and canonical text. The Gate A reference `plan_conditionals` collapsed whitespace only and is a *shape* reference, not a token-invariance reference; packet C's production planner implements this token normalization.
- Maintain a per-container stack. `#if` pushes a frame with one branch; `#elseif`/`#else` append a branch to the top frame; `#endif` pops and finalizes the group.
- Treat `diagnostic` (`#error`/`#warning`) as a dropped non-declaration: it never makes a branch retained.
- Grammar ERROR nodes are already fatal (rule 3); validation below runs on otherwise-clean trees.

### 9.3 Validation (all fatal, even though the grammar accepts them)
- `Unmatched(DirectiveKind, span)` — `else`/`elseif`/`endif` with no open `#if`.
- `DuplicateElse(span)` — a second `#else` in one sequence.
- `ElseIfAfterElse(span)` — `#elseif` after `#else`.
- `MissingEndIf(span)` — open frame at end of container (including a nested open frame).
Evidence: `malformed_directives_are_accepted_by_grammar_but_rejected_by_planner` — every malformed snippet is `PARSE: OK` yet the planner returns the expected `CondError`.

### 9.4 Item kinds, keys, spans, parent chains (exact)
- GROUP → synthetic `ItemKind::Module`. Key: `<container_key>::cond::<guard_token>`, where `guard_token` is the token-normalized header (§9.2) without `#`, first word and condition separated by `:` (`if:DEBUG`, `elseif:os(iOS)` — `os ( iOS )` normalizes to the same token — `else`). For the spike test source, `FILE::cond::if:DEBUG`.
- BRANCH → synthetic `ItemKind::Module`. Key: `<group_key>::branch::<index>::<guard_token>` with zero-based source-order `index` (`FILE::cond::if:DEBUG::branch::0::if:DEBUG`, `…::branch::1::elseif:RELEASE`, `…::branch::2::else`).
- Member inside a branch → normal declaration key `<branch_key>::<kind_token>::<name>`. (The spike uses synthetic `::member::<ordinal>:<node_kind>` keys to prove the shape; C uses real keys.)
- `parent_key`: GROUP → enclosing declaration key, or `None` at file scope; BRANCH → GROUP key; member → BRANCH key; nested GROUP → enclosing BRANCH key. Chains are acyclic and valid under `ProjectedFile::try_new`.
- Spans (zero-based bytes, from the relevant parse node): GROUP from `#if` start to `#endif` end; BRANCH from its header start to the end of the **last source declaration in that branch** (header end when the branch contains no source declaration), independent of projection mode — span computation must not depend on which members a given mode retains; member from its declaration node (attributes excluded). Verified in `conditional_plan_has_exact_keys_spans_and_parent_chains`.

### 9.5 Retention, headers, and empty groups
- Keys are allocated in one **Signatures-superset pass before mode filtering**; a group/branch/member key is identical in Types, Signatures, and Tests.
- A conditional sequence is retained iff at least one branch contains at least one retained member in the requested mode.
- When retained, **all original branch headers are preserved in source order, including branches with no retained members**, so the guard semantics stay correct. An empty branch renders as its header line followed by the next header.
- If no branch has a retained member, the whole GROUP and its BRANCHes are omitted — never emit an empty standalone `#if/#endif`.
- Filter members before composing ancestor canonical text.

### 9.6 Collision policy
`base::KeyAllocator` runs once in source order over the Signatures superset: first use of a base key is unchanged, later collisions append `~1`, `~2`, … . This applies to repeated identical guards in one container, to same-name declarations in one branch, and to callables with identical normalized labels. Ordinals can shift if a colliding sibling is inserted before a declaration; cross-edit identity for same-name siblings is not promised (existing model limitation).

### 9.7 Canonical form
Swift-shaped, no invented braces:
```
#if <guard>
<members at the enclosing declaration indent>
#elseif <guard>
<members at the enclosing declaration indent>
#else
<members at the enclosing declaration indent>
#endif
```
Conditions are never evaluated; only normalized header text is emitted. Conditional directives introduce **no indentation level of their own**: members render at the enclosing container's indent (file scope 0, or +4 inside a type), exactly as the §8.9 goldens show.

## 10. Stable keys and outline invariants

- General shape `<container_key>::<kind_token>::<name>`; top-level container key is the repository path. Conditional keys per §9.4.
- Kind tokens: `type`, `trait`, `impl`, `variant`, `type_alias`, `assoc_type`, `field`, `function`, `method`, `constructor`, `value`, `operator`, `macro`, plus synthetic `cond`/`branch`.
- Callable identity includes normalized external argument labels: `subscribe(to:for:)`, `init(x:y:)`, `subscript(row:col:)`. Same-label overloads use ordinals.
- Extension identity includes normalized target, conformances, and constraints.
- No byte offsets, body text, initializer text, or source formatting in keys.
- Signatures is the superset (`ProjectionMode::superset`); every retained item exists by key in the outline; parent chains valid and acyclic.

## 11. Tests mode rules

- **Swift Testing:** a function carrying `@Test` (final path segment `Test`, including `@Testing.Test` and `@Test(arguments: ...)`) is a test. Any enclosing Swift type holding a surviving test is retained as its container; `@Suite` is optional metadata and alone never makes a member a test (the `@Suite` in the golden below is illustrative, not required). Signatures and normalized attribute arguments are preserved, not bodies.
- **XCTest (syntactic):** method name starts with `test` (non-empty remainder), instance (no `static`/`class`), zero parameters, return type ignored. Container is a `class` directly naming a base whose final path segment is `XCTestCase` (e.g. `XCTest.XCTestCase`), or an `extension` whose target is such a class declared in the same file. No custom-base inference, cross-file resolution, setup/teardown helpers, or generated tests.
- **Filtering:** filter members before composing ancestor canonical text. Retain only containers needed for surviving tests. A conditional GROUP/BRANCH is retained when a descendant branch holds a surviving test; all headers of that sequence are preserved per §9.5. Tests mode declares no types; an empty result does not distinguish "no tests" from "no detection".

Golden (Tests output — one blank line between independent top-level
declarations, single newlines between nested members):
```
@Test func basic() { ... }

@Suite struct MathTests {
    @Test func add(a: Int, b: Int) { ... }
}

final class FooTests: XCTestCase {
    func testThing() { ... }
}

extension FooTests {
    func testExtension() { ... }
}
```

## 12. Evidence: historical Gate A spike files, commands, results

This section records the **Gate A spike only**. It is historical evidence for the frozen decisions, not a statement of the current adapter's tests or crate count. The spike was a probe, not a shipping projector; packet C's production code replaces it.

**Spike files** (only these were edited in the reviewer-fix pass): `crates/lang-swift/src/lib.rs` (parse probe + `plan_conditionals` + `read_in_accessor_position`), `crates/lang-swift/tests/probe.rs` (31 snippets + 14 assertions), plus the two Swift planning docs. Root manifests were already changed in Gate A and were not touched again.

**Commands (historical, recorded at Gate A):**

```
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p lang-swift
  unittests src/lib.rs:      1 passed  (grammar_assigns_to_tree_sitter_027)  # historical Gate A run
  tests/probe.rs:           14 passed, 0 failed                              # historical Gate A run
  doc-tests:                 0
cargo test -p lang-swift --test probe -- --nocapture   # full matrix/printers
cargo check --workspace                                 # 12 crates at Gate A; not the current count
```

Probe highlights:
```
read_alone_clean_misparse: OK   (clean misparse; narrow detector flags 1 span)
read_call_in_getter_body:  OK   (legit _read(1) in getter; detector flags 0)
macro_braced_definition:   ERR  ERROR [1..10]      (invalid/unsupported)
accessors_yielding:        ERR  ERROR [66..130]    (get+set+_read+_modify)
nested_lambda_arguments:   OK   4 lambda_literal nodes (depth stripping)
interpolation_closure:     OK   lambda_literal inside interpolated_expression
conditional_compilation:   OK   flat directives; planner reconstructs + validates
malformed directives:      OK in grammar; planner emits Unmatched / DuplicateElse /
                                ElseIfAfterElse / MissingEndIf
malformed_body/header:     ERR  (grammar ERROR inside stripped body / bad header)
```
Assertion tests: `read_alone_is_a_clean_misparse_detected_narrowly`, `combined_accessors_with_read_are_a_grammar_error`, `braced_macro_definition_is_invalid_unsupported_input`, `accessor_effects_are_visible_as_nodes_or_tokens`, `nested_lambdas_in_attributes_and_defaults_are_all_visible`, `interpolation_contains_executable_closure_nodes`, `malformed_directives_are_accepted_by_grammar_but_rejected_by_planner`, `conditional_plan_has_exact_keys_spans_and_parent_chains`, `conditionals_are_recognized_in_every_declaration_container`, plus the original supported/limitation/inventory printers.

## 13. Handoff (historical Gate A assignments; no approval blockers)

This was the Gate A handoff; all listed work has since landed. B's registration
and a **skeleton** `SwiftProjector` landed first; C replaced that same definition
with the production adapter, and D/E completed fixtures and acceptance/docs.
Neither the spike nor the skeleton was shipping Swift support.

1. **B — registration:** add `Language::Swift`, case-sensitive `.swift` `is_swift()`, JSON `"swift"`, TUI highlighting/icon, exhaustive language matches; widen active schemas, leave archived `codect.diff.v1.json` unchanged.
2. **C — parser/extraction/render:** implement the plan's §§3, 5–10 using `base::doc`/`base::render::LINE_WIDTH`; reproduce `plan_conditionals` and narrow `_read` detection; allocate keys in the Signatures superset.
3. **D — fixtures/invariants:** build goldens from §8 (including conditional retention and long wrapping), not from implementation snapshots.
4. **E — acceptance/docs propagation:** add acceptance coverage and propagate the fifth language to authoritative docs (`PRODUCT.md`, `TECHNICAL_DESIGN.md`, `LANGUAGES.md`, `AGENTS.md`, `README.md`, `docs/agents/*`), summarizing language selection, three modes, no bodies/lambda stripping, conditional guards, test frameworks, and grammar limitations.
5. **Adapter skeleton (historical):** B's skeleton `SwiftProjector` was replaced by C's production adapter; neither the spike nor the skeleton was shipping Swift support.

## 14. Plan status update

`docs/plans/swift-language-support.md` is marked **implemented and validated —
COMPLETE**, with the completion/review record. This contract remains the frozen
behavior target, now implemented and verified; §12 stays historical.
