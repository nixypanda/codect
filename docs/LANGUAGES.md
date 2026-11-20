# Language support

Codect supports five languages: Elm, Haskell, Python, Rust, and Swift. For each
one it projects a selected level of detail — types, signatures, or tests — with
function and value bodies removed. This page is a reference for what is
supported per language and for the quirks to keep in mind.

[`PRODUCT.md`](PRODUCT.md) is authoritative for behavior and scope;
[`agents/PROJECTION.md`](agents/PROJECTION.md) holds the projection model and
[`agents/CONTRACT.md`](agents/CONTRACT.md) the CLI and JSON contract.

## Supported files

Codect selects a language by file extension, using ASCII bytes.

| Language | Extensions | Notes |
| --- | --- | --- |
| Elm | `.elm` | |
| Haskell | `.hs` | Literate Haskell (`.lhs`) is **not** supported. |
| Python | `.py`, `.pyi` | Type stubs are supported like ordinary modules. |
| Rust | `.rs` | |
| Swift | `.swift` | The extension is matched case-sensitively; `.Swift` is not selected. |

Unsupported files are simply not included in focused views. A selected path that
exists but contains no supported files succeeds with empty output. A supported
file selected for projection that fails to parse, or that holds non-UTF-8 bytes,
is a fatal error (exit `1`) with empty stdout — Codect never falls back to raw
source and never emits a partial projection. Move detection may read other
sources best-effort for pairing without projecting them.

## Support matrix

| Capability | Elm | Haskell | Python | Rust | Swift |
| --- | --- | --- | --- | --- | --- |
| Module / file context | ✅ | ✅ | ✅ | ✅ | ✅ |
| Type and alias declarations | ✅ | ✅ | ✅ | ✅ | ✅ |
| Traits, classes, and interfaces | ❌ | ✅ | ✅ | ✅ | ✅ |
| GADTs and unions | ❌ | ✅ | ❌ | ✅ | ✅ |
| Generics and type parameters | ✅ | ✅ | ✅ | ✅ | ✅ |
| Function and method signatures | ✅ | ✅ | ✅ | ✅ | ✅ |
| Values, constants, and statics | ✅ | ✅ | ✅ | ✅ | ✅ |
| Attributes, decorators, and pragmas | ❌ | ✅ | ✅ | ✅ | ✅ |
| Ports and operators | ✅ | ❌ | ❌ | ❌ | ⚠️ |
| Enum members | ❌ | ❌ | ⚠️ | ❌ | ✅ |
| Type-variable declarations | ❌ | ❌ | ⚠️ | ❌ | ✅ |
| Pattern-synonym signatures | ❌ | ⚠️ | ❌ | ❌ | ❌ |
| Test detection | ❌ | ❌ | ✅ | ✅ | ✅ |
| Inferred types | ❌ | ❌ | ❌ | ❌ | ❌ |
| Visibility filtering | ❌ | ❌ | ❌ | ❌ | ❌ |
| Macro-generated declarations | ❌ | ❌ | ❌ | ❌ | ❌ |

Legend: ✅ Full · ⚠️ Partial · ❌ None.

✅ and ❌ mean what they say. Every ⚠️ cell, and every non-obvious behavior
behind a ✅ cell, is explained below.

## ⚠️ Partial support

### Python — enum members

Enum members project as variants — bare names, initializers removed — in both
types and signatures mode. A class is treated as an enum only when one of its
direct base names is `Enum`, `IntEnum`, `StrEnum`, `Flag`, `IntFlag`, or
`EnumMeta` (also as the final segment of a dotted base). A class that inherits
from a custom enum base, or uses a third-party enum library, is projected as an
ordinary class and its members as plain values.

### Python — type-variable declarations

`TypeVar`, `ParamSpec`, `TypeVarTuple`, and `NewType` declarations are
recognized by **call name only**, however the call is qualified. This matches
the standard typing module and its re-exports, but it also matches an unrelated
call that happens to share the name.

### Haskell — pattern-synonym signatures

The projection model and fixtures describe `pattern Syn :: a -> [a]` as part of
signatures mode, but the current build emits neither the signature nor a line in
the outline; a file containing only a pattern synonym projects to its module
header alone. Treat pattern synonyms as unsupported.

### Swift — ports and operators

Swift operator declarations (fixity, name, and named precedence) are projected
in both modes. Swift has no port declaration, so the "ports and operators" cell
is partial. A `precedencegroup` body is metadata and is omitted; its precedence
relationships survive by name on the operator line.

## Quirks

These notes explain behavior that is not obvious from the matrix.

### Elm

- The `module` declaration appears in both types and signatures mode, with its
  `exposing` clause omitted; a `port module` is distinguished as
  `port module Name`.
- An unannotated declaration stays visible and is marked
  `name : <missing type annotation>`. Codect never infers a type.
- Annotations and definitions are paired by declared lower-case name, not by
  adjacency. A standalone annotation with no definition is dropped unless it is
  a `port`.
- A destructuring declaration has no declared lower-case name, so it is omitted.
- A record renders in block form when it is the complete right-hand side of a
  `type alias`; a nested record stays inline while it fits.
- Imports, comments, documentation, bodies, and declarations nested inside
  expressions (`let`, `case`, function bodies) are omitted.

### Haskell

- A file with no module header emits no module line, and the export list is
  omitted.
- The `where` keyword follows a class or instance header only when a member is
  actually shown.
- A definition covered by an explicit signature is represented by the signature
  alone, regardless of source order. An unannotated function shows its written
  head; an unannotated simple binding shows just its name.
- A class or instance member is keyed by its written method name, so two members
  with the same name in one container collide and receive a source-order
  ordinal.
- Pragmas (`{-# LANGUAGE ... #-}`, `{-# INLINE ... #-}`, and similar) are
  preserved; a pragma prefixes the declaration that follows it, and a
  file-level pragma prefixes the module header.
- Bindings inside `where` and `let` are never inspected. Template Haskell
  splices and CPP directives are not projected.

### Python

- A class that retains no members renders as `class X: ...` (keeping its bases
  and type parameters), so an empty class is still visible.
- A class-level annotated attribute is a field (both modes); a class-level
  unannotated attribute is a value (signatures only); an enum member is a bare
  name with its initializer removed (both modes).
- A module-level annotated value is a signature, not a type declaration; a
  module-level unannotated value renders as its bare name.
- A module- or class-level docstring is recognized only when the string is the
  sole statement in that body. Docstrings, comments, and imports are omitted.
- Decorators are preserved because they can change a declaration's meaning.
- `.pyi` stubs project like `.py`; a stub's `...` bodies are already the
  canonical placeholder.
- Unannotated declarations are rendered exactly as written; Python does not use
  Elm's missing-annotation placeholder.

### Rust

- An out-of-line `mod name;` has no content in this file and produces nothing.
  The referenced file is projected independently by its own path — Codect does
  not resolve Cargo module rules.
- A trait implementation header survives types mode even when it has no
  associated type, because the header is a type relationship. An inherent
  `impl` block with no included type information, and an `extern` block that
  retains no member, are dropped entirely.
- Non-documentation outer attributes are preserved, including `#[cfg(test)]`,
  `#[test]`, and `#[deprecated]`. Doc comments and `#[doc = ...]` attributes are
  removed.
- Declarative, procedural, derive, and attribute macros are never expanded; a
  macro definition and a macro invocation each produce no items.
- Closures and local items are excluded, and a body — including a constant or
  static initializer — is never inspected.
- Bodies become `;` and initializers are removed; parameter patterns are kept
  because they are part of the written declaration. `use` and `extern crate`
  declarations are omitted.
- Attribute arguments are normalized, so
  `#[test_case(-2, -4 ; "both negative")]` renders as
  `#[test_case(-2, -4; "both negative")]`.

### Swift

- A Swift file is selected only by the case-sensitive `.swift` extension. No
  Apple SDK, Swift compiler, SourceKit, dependency resolution, or macro
  expansion is needed; parsing and projection are compiler-independent.
- Types mode retains the shape of structs, classes, actors, and enums (including
  nested types), protocols (inheritance, generics, associated types, and typed
  property requirements), conforming extensions, extensions with type-level
  members, enum cases with associated-value types and labels,
  `typealias`/`associatedtype`, typed properties, and operators. Signatures adds
  callable declarations and requirements, free functions, global `let`/`var`,
  initializers, `deinit`, `subscript`, and untyped properties. A conformance
  header survives in Types even with no visible members, but a non-conformance
  extension holding only methods/values is omitted from Types and retained in
  Signatures; an empty non-conformance extension is omitted in every mode.
- Bodies are never copied. A concrete `func`/method/`init`/`deinit` ends in a
  `{ ... }` placeholder; protocol requirements have no placeholder; a computed
  property renders specifiers and effects only
  (`var all: [Item] { get async throws }`); a `subscript` likewise keeps its
  accessor interface and effects (`subscript(i: Int) -> Int { get set }`,
  including setter-only subscripts) and never renders accessor bodies;
  observers render `{ willSet didSet }`; stored properties and global values drop
  their initializer and carry only a written type. Written annotations are
  preserved as written, including opaque ones such as `var body: some View`; no
  type is fabricated, so a genuinely untyped binding or global renders as its
  name alone.
- Each binding pattern in a multi-binding declaration is one projected item:
  global bindings are `Value`s and type-level property bindings are `Field`s.
  Simple names produce separate items, while a tuple pattern produces one item
  named by its canonical written pattern (`(a, b)`), so the stable key stays
  non-empty. The tuple annotation stays on the tuple pattern; a shared trailing
  annotation propagates only through preceding uninitialized simple bindings of
  the same declaration (`var x, y: Int`), never onto an initialized untyped
  binding (`let c = 3`) and never across an initialized binding. The initializer
  is dropped, no type is inferred, and each item's span covers the complete
  binding including its stripped implementation.
- Default arguments are part of the interface and are retained, but **every**
  closure inside an attribute argument or default expression — at any nesting
  depth — is replaced by `{ ... }`. Literal text is preserved, including string
  interpolation punctuation, while executable closures inside interpolation are
  stripped the same way.
- Attributes and modifiers are token-normalized in source order and never depend
  on source whitespace. One attribute renders inline on the declaration line
  when it fits; two or more render one per line before the declaration.
- Conditional-compilation guards are never evaluated or dropped. All original
  `#if`/`#elseif`/`#else` headers of a retained sequence are preserved in source
  order, including branches with no retained member; conditions are normalized
  by tokens, so `os(iOS)` and `os ( iOS )` are the same guard, and comments are
  omitted. Directives add no indentation of their own — members keep the
  enclosing container's indent. A sequence with no retained member is dropped
  entirely. Malformed sequences (unmatched or duplicate directives, `#elseif`
  after `#else`, missing `#endif`) are a fatal error even when the grammar
  accepts them.
- A macro declaration's source-visible signature is retained and its definition
  is stripped; macro invocations such as `#Preview`/`#expect` never synthesize
  declarations. Macro-generated declarations are not projected.
- Effects written as anonymous tokens (`async`, typed `throws(E)`,
  `mutating`/`nonmutating`, isolation modifiers) are captured from source tokens
  with canonical ordering such as `mutating get nonmutating set`.
- **Grammar limitations.** Parsing is all-or-nothing, with fatal errors for any
  syntax error — including inside a stripped body. An accessor-position
  `_read { ... }` is not modeled and is fatal; ordinary `_read()` calls and
  `_read(...)` calls inside a real getter body are allowed. Braced macro
  definitions are invalid input and fatal. There is no macro expansion, type
  inference, cross-file resolution, or public-visibility filtering.

## Test detection

Tests mode is the signatures projection restricted to test declarations. It
declares no types, so it is **not self-contained**: it can show
`fn parse_empty() -> Result<Err, ParseErr>;` without declaring `Result` or
`ParseErr`, and a type change is invisible to it.

| Language | A test is | Retained containers | Not detected |
| --- | --- | --- | --- |
| Rust | a function with a primary marker (`#[test]`, `#[tokio::test]`, `#[rstest]`, `#[test_case]`, `#[test_matrix]`) | a `mod` holding a test | macro-generated cases, `#[bench]`, a `#[test]` method inside an `impl` |
| Python | a function named `test_*` | a class holding a test method | fixtures and helpers |
| Swift | a function carrying `@Test` (including `@Testing.Test` and `@Test(arguments: ...)`), or a `test*` instance method in a same-file `XCTestCase` class/extension | any enclosing type holding a surviving test (a `@Suite` is optional, not required), an XCTest class/extension holding a test, and a conditional group holding a test | `@Suite` alone, setup/teardown helpers, custom bases, cross-file inheritance, generated tests |
| Elm | — | none | everything (no detection) |
| Haskell | — | none | everything (no detection) |

A Rust attribute is matched on its **final path segment**, so a qualified path
such as `#[tokio::test]` matches. Companion attributes (`#[should_panic]`,
rstest's `#[case(...)]`) never mark a test on their own. A Python class is
retained only when one of its methods is a test, which covers both the pytest
`Test*` convention and `unittest.TestCase`.

Swift detection is **syntactic**: a `@Test` marker matches on its final path
segment, and `@Suite` alone never makes its members tests. Any enclosing Swift
type holding a surviving test is retained as its container; `@Suite` is optional
metadata, not a requirement. XCTest detection requires a
`test` prefix with a non-empty remainder, an instance method (not `static` or
`class`), zero parameters, and a `class` that directly names a base whose final
path segment is `XCTestCase`, or an `extension` whose target is such a class
declared in the same file. There is no custom-base inference, cross-file
inheritance resolution, setup/teardown helper detection, or generated test
detection. Members are filtered before ancestor text is composed.

## Cross-cutting quirks

- **Bodies never appear.** Every mode strips function, method, and value bodies;
  only a declared type or signature is shown.
- **Canonical output.** Comments, documentation, whitespace, and formatting
  never affect output, so a formatting-only or body-only edit disappears from a
  focused diff for an unmoved file. Inside a detected move it produces no hunks,
  but the move remains visible.
- **Fixed line width.** Line breaks are fixed by declaration shape and a single
  80-column budget (`base::LINE_WIDTH`); terminal width never changes output.
  Elm and Haskell break lists with leading commas and no trailing comma; Python,
  Rust, and Swift emit a trailing comma only when a list breaks.
- **Visibility is not filtered.** A types view shows public and private type
  declarations alike; public-interface filtering is not implemented.
- **Empty vs unavailable.** Elm and Haskell retain nothing in tests mode, so an
  empty tests projection does not distinguish "detection unavailable" from "no
  tests". Rust, Python, and Swift do detect tests; for them an empty result means
  no test was detected. Tests mode declares no types in any language.
- **Ordering and omission.** Show files and scope traversal use raw
  repository-path byte order. Diff events are keyed and ordered by target path,
  using the sole present path for additions and deletions. A diff omits an
  unmoved file whose two projections are equal. A detected file move with at
  least one nonempty projection remains visible even when its projections are
  equal; a move with two empty requested-mode projections is omitted.
- **Fatal on selected projection failure.** A supported file selected for
  projection that fails to parse, or that is not valid UTF-8, fails the whole
  command: empty stdout, one diagnostic on stderr, no partial projection. Move
  detection may inspect additional off-scope sources. A fuzzy candidate with
  unknown size, more than 256 KiB, invalid UTF-8, or a read failure is
  fuzzy-ineligible, not fatal. Exact matching still considers every candidate
  and all sizes using raw bytes where needed; an unreadable worktree candidate
  cannot match exactly.

## Not supported

- Public-interface filtering (the planned Public mode).
- Complete source viewing or raw source diffs (the planned Full mode).
- Inferred types for unannotated declarations.
- Function bodies, or local and nested function signatures.
- Implementation-change indicators.
- Import, comment, or documentation views.
- Macro expansion, or declarations generated by macros.
- Declaration-level move detection or copy detection (file moves are detected).
- Swift: macro expansion, inferred types, cross-file type or XCTest-base
  resolution, accessor-position `_read { ... }`, and braced macro definitions.
- Languages other than Elm, Haskell, Python, Rust, and Swift.

## See also

- [Projection model](agents/PROJECTION.md) — modes, per-language rules, stable
  keys.
- [CLI and JSON contract](agents/CONTRACT.md) — flags, exit codes, schemas.
- [`PRODUCT.md`](PRODUCT.md) — authoritative behavior and scope.
- [`TECHNICAL_DESIGN.md`](TECHNICAL_DESIGN.md) sections 11, 12, 23, and 24.
