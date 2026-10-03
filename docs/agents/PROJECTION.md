# Projection model

Audience: agents and other machine consumers that need to reason about what
Codect emits. `PRODUCT.md` is authoritative; `TECHNICAL_DESIGN.md` sections
5–13 and 23–24 hold the full rules.

## Modes

Codect has two implemented projection modes.

| Mode | Includes |
| --- | --- |
| `types` | Type declarations only: the shape of the code. |
| `signatures` | Everything in `types`, plus every named function, method, and value signature. |

Two more modes are planned but not implemented: `public` (public interface
filtering) and `full` (complete source). The Neovim plugin has a local `full`
fold state that opens the real buffer; that is not the planned `full` mode.

Function and value **bodies are never projected**. Implementation-only changes
are intentionally invisible in both modes and in both diff directions.

## What every projection guarantees

- Emission is deterministic and independent of the current directory, locale,
  terminal width, wall clock, environment variables, or installed compilers.
- Original whitespace and comments do not affect output. Canonical rendering is
  what makes formatting-only edits disappear from a focused diff.
- Declaration order, semantic token order, member order, visibility, generic
  parameters, constraints, and selected modifiers are preserved.
- Line breaks are fixed by declaration shape and a single 80-column budget
  (`base::LINE_WIDTH`). Terminal width never changes output.
- Exactly one trailing newline per projected file; one blank line between
  top-level declarations.

## Per-language summary

### Elm

- Types: `type` declarations with parameters and every constructor; `type alias`
  with its complete right-hand side and record fields.
- Signatures: adds every top-level function and value, its matching explicit
  annotation, ports, and infix declarations.
- An unannotated declaration stays visible and is marked
  `<missing type annotation>`. Codect never infers a type.
- Omitted: imports, comments, docs, bodies, nested `let` declarations.

### Rust

- Types: named/tuple/unit structs, enums and variants, unions, aliases, trait
  headers (supertraits, associated types and constraints), trait implementation
  headers with associated type assignments, and their generics and `where`
  clauses.
- Signatures: adds free functions, inherent and trait methods, associated
  functions, foreign declarations, constants and statics (declared types only),
  and signature modifiers such as `async`, `const`, `unsafe`, and `extern`.
- Bodies become `;`; initializers are removed.
- Non-documentation outer attributes are preserved; doc comments are not.
- Omitted: `use`, `extern crate`, macros and expansion, closures, local items.

### Haskell

- Types: `data`/`newtype` with constructors in prefix, record, infix, and GADT
  form; `deriving` clauses; `type` synonyms; type and data families and
  instances; kind signatures and type roles; class and instance headers.
- Signatures: adds top-level type signatures (including multi-name),
  unannotated binding heads, class and instance method signatures/heads, foreign
  imports, and pattern-synonym signatures.
- Pragmas are preserved; imports, comments, and Haddocks are omitted.
- Omitted: bindings inside `where` and `let`.

### Python

- Types: `class` declarations with bases, keywords, and PEP 695 type parameters;
  class-level annotated attributes; enum members; `type` aliases; and
  `TypeVar`/`ParamSpec`/`TypeVarTuple`/`NewType` declarations.
- Signatures: adds `def`/`async def` signatures with parameters, annotations,
  defaults, return types, decorators, and type parameters; module- and
  class-level values with declared types.
- Bodies become the `...` placeholder; initializers are removed.
- Decorators are preserved; imports, comments, and docstrings are omitted.

## Stable keys

A stable key identifies a declaration inside its file. It is not a global ID and
is not displayed by default. Typical forms:

- Elm/Haskell module or top-level item: module name, item kind, declared name.
- Rust file module: repository path; inline module: parent key plus name.
- Rust inherent impl: normalized target type; trait impl: normalized trait path
  plus normalized target type.
- Python file module: repository path; class member: class key, kind, name.

Keys never include byte offsets, so an edit before a declaration does not
destabilize it. A collision appends a deterministic source-order ordinal.
`base::KeyAllocator` is the single implementation.

## The outline and the superset rule

The editor surface (`codect.show.v1`) pairs the requested mode's projection with
a **mode-independent outline** built from the Signatures projection, which is a
superset of Types by `stable_key`. `retained_in_mode` tells a consumer whether
the requested mode keeps each declaration. This is what lets one buffer fold at
two depths: a retained declaration shows its mode-correct canonical fragment; a
dropped declaration shows the outline signature.

## See also

- [CLI and JSON contract](CONTRACT.md).
- [Invariants](INVARIANTS.md).
- [`docs/PRODUCT.md`](../PRODUCT.md).
- [`docs/TECHNICAL_DESIGN.md`](../TECHNICAL_DESIGN.md) sections 5–13, 23–24.
