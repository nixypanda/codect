# Language support

Codect supports four languages: Elm, Haskell, Python, and Rust. For each one it
projects a selected level of detail — types, signatures, or tests — with
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

Unsupported files are simply not included in focused views. A selected path that
exists but contains no supported files succeeds with empty output. A supported
file that fails to parse, or that holds non-UTF-8 bytes, is a fatal error (exit
`1`) with empty stdout — Codect never falls back to raw source and never emits a
partial projection.

## Support matrix

| Capability | Elm | Haskell | Python | Rust |
| --- | --- | --- | --- | --- |
| Module / file context | ✅ | ✅ | ✅ | ✅ |
| Type and alias declarations | ✅ | ✅ | ✅ | ✅ |
| Traits, classes, and interfaces | ❌ | ✅ | ✅ | ✅ |
| GADTs and unions | ❌ | ✅ | ❌ | ✅ |
| Generics and type parameters | ✅ | ✅ | ✅ | ✅ |
| Function and method signatures | ✅ | ✅ | ✅ | ✅ |
| Values, constants, and statics | ✅ | ✅ | ✅ | ✅ |
| Attributes, decorators, and pragmas | ❌ | ✅ | ✅ | ✅ |
| Ports and operators | ✅ | ❌ | ❌ | ❌ |
| Enum members | ❌ | ❌ | ⚠️ | ❌ |
| Type-variable declarations | ❌ | ❌ | ⚠️ | ❌ |
| Pattern-synonym signatures | ❌ | ⚠️ | ❌ | ❌ |
| Test detection | ❌ | ❌ | ✅ | ✅ |
| Inferred types | ❌ | ❌ | ❌ | ❌ |
| Visibility filtering | ❌ | ❌ | ❌ | ❌ |
| Macro-generated declarations | ❌ | ❌ | ❌ | ❌ |

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

## Test detection

Tests mode is the signatures projection restricted to test declarations. It
declares no types, so it is **not self-contained**: it can show
`fn parse_empty() -> Result<Err, ParseErr>;` without declaring `Result` or
`ParseErr`, and a type change is invisible to it.

| Language | A test is | Retained containers | Not detected |
| --- | --- | --- | --- |
| Rust | a function with a primary marker (`#[test]`, `#[tokio::test]`, `#[rstest]`, `#[test_case]`, `#[test_matrix]`) | a `mod` holding a test | macro-generated cases, `#[bench]`, a `#[test]` method inside an `impl` |
| Python | a function named `test_*` | a class holding a test method | fixtures and helpers |
| Elm | — | none | everything (no detection) |
| Haskell | — | none | everything (no detection) |

A Rust attribute is matched on its **final path segment**, so a qualified path
such as `#[tokio::test]` matches. Companion attributes (`#[should_panic]`,
rstest's `#[case(...)]`) never mark a test on their own. A Python class is
retained only when one of its methods is a test, which covers both the pytest
`Test*` convention and `unittest.TestCase`.

## Cross-cutting quirks

- **Bodies never appear.** Every mode strips function, method, and value bodies;
  only a declared type or signature is shown.
- **Canonical output.** Comments, documentation, whitespace, and formatting
  never affect output, so a formatting-only or body-only edit disappears from a
  focused diff.
- **Fixed line width.** Line breaks are fixed by declaration shape and a single
  80-column budget (`base::LINE_WIDTH`); terminal width never changes output.
  Elm and Haskell break lists with leading commas and no trailing comma; Python
  and Rust emit a trailing comma only when a list breaks.
- **Visibility is not filtered.** A types view shows public and private type
  declarations alike; public-interface filtering is not implemented.
- **Empty vs unavailable.** Elm and Haskell retain nothing in tests mode, so an
  empty tests projection does not distinguish "detection unavailable" from "no
  tests".
- **Ordering and omission.** Files are processed in raw repository-path byte
  order, and a diff omits any file whose two projections are equal.
- **Fatal on failure.** Any supported file that fails to parse, or that is not
  valid UTF-8, fails the whole command: empty stdout, one diagnostic on stderr,
  no partial projection.

## Not supported

- Public-interface filtering (the planned Public mode).
- Complete source viewing or raw source diffs (the planned Full mode).
- Inferred types for unannotated declarations.
- Function bodies, or local and nested function signatures.
- Implementation-change indicators.
- Import, comment, or documentation views.
- Macro expansion, or declarations generated by macros.
- Rename or move detection beyond what is visible in the projected text.
- Languages other than Elm, Haskell, Python, and Rust.

## See also

- [Projection model](agents/PROJECTION.md) — modes, per-language rules, stable
  keys.
- [CLI and JSON contract](agents/CONTRACT.md) — flags, exit codes, schemas.
- [`PRODUCT.md`](PRODUCT.md) — authoritative behavior and scope.
- [`TECHNICAL_DESIGN.md`](TECHNICAL_DESIGN.md) sections 11, 12, 23, and 24.
