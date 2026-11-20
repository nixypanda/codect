# Work item: Swift language support

Status: **Implemented and validated — COMPLETE** on `feat/swift-language-support`.
The frozen behavior target is `docs/plans/swift-language-contract.md` (now
marked implemented/verified). All packets (B registration/schema/TUI, C
parser/extraction/render, D fixtures/invariants, E engine/CLI/editor acceptance
and documentation) have landed and been integrated. Both independent reviewers
(F parser/projection, R invariants/integration) returned GO with no remaining
findings, and QA final pass 2 is green. See “Completion and review record”
below. No open approval blockers remain.

> Gate A result: implementation agent A validated `tree-sitter-swift` 0.7.4
> against Tree-sitter 0.27 (ABI 15), probed modern Swift syntax including bodies,
> and produced `docs/plans/swift-language-contract.md` with frozen behavior,
> grammar limitations, goldens, and evidence. After the Gate A review, the
> contract and spike were updated to reconstruct and validate conditional
> compilation (guards are never dropped), detect `_read` only in accessor
> position, retain typed protocol properties and accessor effects, strip nested
> closures/interpolation bodies, and define wrapping via `base::doc`. The spike
> lives in `crates/lang-swift` (branch `feat/swift-language-support`); it is not
> a shipping projector. Do not start packets B–E before reading that contract.

## 1. Outcome and boundaries

Add Swift as a fifth language, selected by the case-sensitive `.swift`
extension, with deterministic Types, Signatures, and Tests projections. Preserve
the existing read-only, compiler-independent, all-or-nothing behavior. Existing
languages must retain byte-for-byte output.

Use a new `crates/lang-swift` adapter and the existing engine, document renderer,
outline, Git snapshot, and move-detection machinery. Do not introduce SwiftSyntax,
SourceKit, an installed Swift compiler, dependency resolution, macro expansion,
type inference, or public-visibility filtering.

This document is an execution plan, not authorization to start implementation.
When implementation is requested, use the implementation and independent review
subagents described below. Do not launch them during the planning-only task.

## 2. Read first

- `AGENTS.md`, `docs/PRODUCT.md`, `docs/TECHNICAL_DESIGN.md`
- `docs/agents/{CONTRACT,PROJECTION,INVARIANTS}.md`
- `crates/base/src/{model,language,keys,outline,doc}.rs`
- `crates/lang-rust/src/` and `crates/lang-python/src/`, including their tests
- `crates/engine/src/engine.rs` and `crates/engine/tests/outline.rs`
- `crates/cli/src/json.rs` and active schemas in `docs/schema/`
- `docs/CONTRIBUTING.md`

Inspect the current tree before editing; other changes may have landed since
this plan. Preserve unfamiliar changes. The coordinator owns cross-task edits
and integration; agents must not overwrite each other's files.

## 3. Gate A: grammar feasibility and an executable contract

Assign implementation agent A a parser/contract spike before parallel work.

1. Evaluate `tree-sitter-swift` (0.7.4 was available when planning) with the
   workspace's Tree-sitter 0.27. Verify the actual grammar ABI and compiled
   parser assignment; dependency metadata alone is not proof.
2. Exercise syntax in declaration headers AND stripped bodies: SwiftUI result
   builders, macros, property wrappers, concurrency/isolation, typed throws,
   ownership modifiers, parameter packs, regex/string literals, and conditional
   compilation. Record supported and unsupported syntax with minimized fixtures.
3. Check parser build portability, license, dependency footprint, and practical
   parse/build cost. Normal execution must need no Apple SDK or Swift tooling.
4. Produce a syntax-node/field inventory and exact expected output fixtures for
   the projection decisions below. Freeze item kinds, placeholders, stable keys,
   conditional containers, and test rules before dependent agents start.
5. Update authoritative product/design documentation alongside the contract.
   Do not make the implementation silently define product behavior.

Gate: a compiling parser spike, a documented syntax baseline, and approved
goldens for every declaration family. If ordinary target Swift code fails to
parse, propose a pinned grammar fix or reduced scope for approval. Never mask
grammar errors, fall back to raw source, or relax global failure behavior.

### Proposed projection contract to resolve at Gate A

These are the intended defaults; any necessary deviation must be explicit.

| Construct | Types | Signatures |
| --- | --- | --- |
| struct/class/enum/actor, including nested types | Header and retained type members | Also callable/value members |
| protocol | Inheritance, generics, associated types, type-level members | Also requirements and signatures |
| enum case | Name and associated-value types/labels; strip raw-value initializer | Same |
| typealias / associatedtype | Written type, bounds, defaults, constraints | Same |
| extension | Header when expressing conformance or retaining type members | Header and retained members; omit empty non-conformance extensions |
| stored/computed instance or type property | Explicitly typed member shape | Member shape, including untyped declarations without inferred types |
| global let/var | Omit | Name and written type, no initializer |
| func/init/deinit/subscript | Omit | Full written declaration interface, no implementation |
| import/comments/docs/local declarations | Omit | Omit |

- Preserve access control, setter access, attributes, modifiers, inheritance,
  generic parameters/constraints, parameter labels/names/defaults, return types,
  effects, and relevant accessor effects/modifiers. Default arguments are part of
  the interface, not ordinary variable initializers.
- Stored properties and computed properties must have an explicit canonical
  policy: retain read/write interface and accessor effects, but strip getter,
  setter, `_read`/`_modify`, and observer implementations. Do not fabricate a type
  for `var body: some View` or for an unannotated binding. Decide treatment of
  observers, multi-bindings, tuple patterns, and protocol property requirements.
- Canonical bodies are projection placeholders, not necessarily compilable Swift.
  Freeze one representation per family; never copy executable bodies as a shortcut.
- Preserve operator names/fixity and precedence relationships where represented
  as declarations; decide the existing ItemKind mapping explicitly.
- Retain conditional branches containing projected declarations, with normalized
  `#if`/`#elseif`/`#else` guards; do not evaluate conditions. Define deterministic
  container keys, nesting, spans, empty-branch behavior, and outline membership.
- Preserve macro attributes as declaration metadata without expansion. Decide
  whether named macro declarations have a source-visible signature; freestanding
  invocations never synthesize declarations. Declaration argument rendering must
  not leak closure bodies through attributes or default expressions; state a
  placeholder rule and test it against existing no-bodies guarantees.
- Reuse existing ItemKinds where accurate: Type, Trait, TraitImplementation,
  AssociatedType, Field, Variant, Function, Method, Constructor, Value, Operator.
  Do not add kinds just for spelling differences. Any new kind/container requires
  an explicit JSON/schema/client compatibility audit.
- Stable keys use path, parent context, kind, and declared name; callable identity
  includes normalized external argument labels, distinguishing common overloads.
  Use `KeyAllocator` for remaining collisions, including same-label overloads.
  Extension identity includes normalized target, conformances, and constraints.
  No byte offsets, body text, or source formatting in keys. Document ordinal
  limitations rather than promising cross-edit identity the model cannot provide.
- Every retained item must exist by key in the Signatures outline. Key allocation
  must be mode-independent, including when dropped siblings collide. Parent
  chains and canonical fragments must match the existing model's invariants.

### Tests mode

Target both Swift Testing and syntactic XCTest detection:

- A function carrying `@Test` (including qualified spelling) is a test; preserve
  its signature and normalized attribute arguments, not its body.
- XCTest detection requires a `test*` instance method in a class directly naming
  `XCTestCase` as a base (including qualification), or an extension whose target
  is such a class declared in the same file. Freeze parameter/static exclusions
  and name-prefix semantics with examples.
- Retain only containers needed for surviving tests, including conditional
  context; `@Suite` alone does not make every member a test.
- No cross-file inheritance resolution, custom-base inference, setup/teardown
  helpers, or generated tests. Document these limits.
- Filter members before composing ancestor canonical text, not after extraction.

## 4. Implementation packets and ownership

After Gate A, the coordinator commits to a dependency order and assigns these
packets to separate implementation subagents. Independent packets may run in
parallel; dependent ones must wait for their interfaces to be frozen.

### B: shared registration and integration skeleton

Own root Cargo manifests/lockfile, `crates/base`, engine registration/dependency
wiring, `crates/cli/src/json.rs`, active schemas, and exhaustive TUI language
matches. Add `.swift` detection/path tests, `Language::Swift`, projector wiring,
JSON `swift`, syntax highlighting, and icon handling.

Create the minimal adapter crate entry point agreed with C, then transfer
adapter ownership. If a temporary skeleton is needed, keep it on the working
branch only; do not ship empty-success support for Swift files. Leave archived
`codect.diff.v1.json` unchanged. Inspect all exhaustive language matches and
hard-coded supported-language lists, not only known locations.

### C: Swift parser, extraction, and canonical rendering

Own `crates/lang-swift/src/` after B's skeleton handoff. Implement `syntax.rs`,
`extract.rs`, `render.rs`, and `SwiftProjector` using the existing shared document
and key helpers where appropriate. Centralize grammar strings; fail on error or
missing nodes anywhere in selected source. Preserve UTF-8 source spans and nested
item structure. Traverse declaration containers, never executable bodies.

Implement Types/Signatures before Tests, but preserve mode-independent keys from
the start. Use structural/token-aware canonicalization, not regex-based body
stripping or raw source slicing that preserves incidental whitespace. Apply the
fixed 80-column budget, deterministic indentation, and trailing-newline rules.

### D: independent fixtures and invariant tests

Own `fixtures/swift/` and `crates/lang-swift/tests/`. Start after Gate A and B's
crate skeleton, in parallel with C. Write expected output from the agreed
contract, not by accepting implementation snapshots. Include:

- All declaration families, empty/nested containers, conformances, constrained
  extensions, overloads, Unicode/escaped identifiers, and multiline signatures.
- Nested generic/function/tuple/optional/existential/opaque/metatype types;
  attributes, defaults, strings with comment-looking text, and long wrapping.
- Accessors/observers, wrappers, macros, conditional branches, and test frameworks.
- Equal projections after body, initializer, comment, and formatting edits;
  changed projections after relevant type/signature/modifier/guard edits.
- Stable keys after preceding edits, collisions and filtered sibling collisions;
  source-span correctness and valid parent chains in every mode.
- Malformed headers AND malformed stripped bodies; no partial projection.

### E: engine/CLI/editor acceptance and documentation

Own new Swift-focused engine/CLI integration tests, editor fixtures/tests as
needed, and language-facing docs after Gate A's contract edits. Coordinate shared
existing test-file edits with B rather than editing them concurrently.

Exercise disk show, stdin show, committed revisions, `:index`, `:worktree`,
`:empty`, path/area selection, text/JSON, exact/fuzzy moves, untracked files,
body-only empty diffs, and atomic failure with another valid file selected.
Check same-language move pairing and mixed-language repositories.

Verify Types/Tests outline membership, source navigation, folding, JSON language
acceptance, and highlighting through TUI, Neovim, and DSH where supported. Do not
assume clients need code changes; prove acceptance and add focused coverage.
Update `AGENTS.md`, README, product/design, language reference, agent docs, and
other current four-language claims. Avoid unrelated documentation cleanup.

## 5. Independent adversarial reviews

Reviewers must be different subagents from implementers. They report findings
with severity, exact locations, minimal reproductions, expected versus actual
behavior, and missing tests. They should not simply rerun golden tests or modify
production code; fixes go back to the owning implementer/coordinator.

1. **Gate A reviewer: contract and grammar skeptic.** Before the contract freezes,
   challenge syntax coverage, conditional representation, no-bodies promises,
   test false positives, ItemKind mappings, and schema compatibility. Identify
   ambiguous choices that would force downstream rework.
2. **Parser/projection reviewer.** After C/D integrate, independently construct
   valid and invalid Swift inputs. Attack body leakage via accessors, closures,
   defaults, macros, strings, nested declarations, and observer bodies. Challenge
   modern syntax and failure behavior, especially errors inside stripped bodies.
3. **Invariants/integration reviewer.** In parallel with reviewer 2, attack key
   collisions across modes, source spans, duplicate containers, conditional
   nesting, JSON/schema compatibility, mixed-file atomicity, snapshots/moves,
   compiler independence, and regressions to the original four languages.

The coordinator triages every finding. Fix release-blocking correctness issues,
add regression tests, and return fixes to the relevant independent reviewer.
Do not label a finding resolved without a passing reproduction or a documented,
approved contract limitation. Repeat until no unresolved critical/high findings
remain; explicitly list any accepted lower-severity limitations.

## 6. Gates, checks, and completion

Suggested order:

1. A spike -> independent contract review -> Gate A freeze.
2. B skeleton/registration -> C and D in parallel; E starts compatible tests/docs.
3. Coordinator integration -> focused adapter/engine/CLI/editor tests.
4. Independent adversarial reviewers 2 and 3 in parallel -> fixes -> re-review.
5. Final full checks and acceptance report.

Required final check: `just check`. Also run `just test-nvim` when exercising the
Neovim integration, and applicable platform/package checks for the new parser.
Record failures caused by unavailable tooling separately from product failures;
never report unrun checks as passing. Review the final diff and dependency tree.

Done means:

- Swift types/signatures/tests match the frozen, documented contract.
- No bodies/initializers leak; no raw-source fallback or repository execution.
- Outline/key, formatting/body invariance, fatal-error atomicity, and move
  semantics have direct regression coverage.
- Existing language goldens remain unchanged unless separately authorized.
- Active JSON schemas and consumers accept Swift without a gratuitous schema
  version change; any genuinely incompatible model change is explicitly resolved.
- All required checks pass, independent findings are dispositioned, and syntax
  limitations are stated in the user-facing language reference.

## 7. Planning estimate

Allow roughly 2–3 engineer-weeks for the baseline, including review and hardening:
1–2 days for grammar/contract, 1–2 for wiring, 4–6 for adapter work, and 3–5 for
fixtures/integration/review (some overlap). This is not a guarantee. Grammar
patches, new shared item kinds, or broad modern-Swift gaps require re-estimation
at Gate A rather than silent scope expansion.

## 8. Completion and review record

**Deliverables.** A production `SwiftProjector` in `crates/lang-swift` built on
`tree-sitter-swift` 0.7.4 (grammar ABI 15, Tree-sitter 0.27; no Apple SDK, Swift
compiler, or SourceKit), case-sensitive `.swift` detection and `Language::Swift`,
engine registration, `"swift"` in the active JSON schemas without a version
bump, TUI highlighting/icon and exhaustive language matches, Swift fixtures and
invariant tests, engine/CLI/Neovim/DSH acceptance coverage, and fifth-language
documentation across product, design, language reference, and agent docs.

**Review dispositions.** Gate A's contract/grammar blockers were resolved before
implementation and remain recorded in the frozen contract. Independent reviewer
F examined 142 valid Swift cases, 32 invariance pairs, and 26 targeted
ternary/nearby rechecks with no executable-body leak; all high- and
medium-severity findings were fixed and re-verified, including F4's clarification
of extension retention intent without widening scope. Independent reviewer R
returned GO with no remaining findings; stale “Rust and Python only” Tests prose
in help/schema descriptions was corrected, and the final low-severity ternary
spacing issue was fixed and independently verified. No critical or high findings
remain open.

**Validation.** In `nix develop`, with the local toolchain shims bypassed using
environment only (no project configuration changed):

```sh
export PATH="$HOME/.cargo/bin:$PATH"
export CC="$NIX_CC_FOR_BUILD/bin/clang"
export CXX="$NIX_CC_FOR_BUILD/bin/clang++"
export AR="$NIX_CC_FOR_BUILD/bin/ar"
export RANLIB="$NIX_CC_FOR_BUILD/bin/ranlib"
export RUSTFLAGS="-C linker=$NIX_CC_FOR_BUILD/bin/clang"

just check          # EXIT 0
just test-nvim      # 64 passed, 0 failed
just test-dsh       # 54 passed, 0 failed
git diff --check    # clean
```

QA final pass 2 also confirmed the branch is `feat/swift-language-support`, the
589-file content aggregate is byte-identical before and after
(`66f047227d1650ee736011ab2a4df6d889f9e44b52ff5ad4c75e418932af233a`), the final
render hash is `5553de69a3104d65773c925200a5e14c03c8161c92d507fd8fee084ec806064d`,
the original four language adapters, shared renderer and tracked goldens are
unchanged, and the archived `codect.diff.v1.json` is untouched. Full pass 2 logs
are in `/private/var/folders/ky/lktlhg4x5v11fg0jydm7fkrm0000gn/T/opencode/codect-final-validation/pass2/`.
The untracked user file `docs/plans/file-move-detection.md` was preserved.

**Intentional, tested limitations.** An accessor-position `_read { ... }` is
unsupported and fatal (an ordinary `_read()` call is allowed); macros are never
expanded and their definitions are stripped; no types are inferred from
initializers; XCTest bases are resolved only in the same file (no cross-file
inheritance or custom-base inference); visibility is not filtered; conditional
guards are preserved but never evaluated. These are documented in
`docs/LANGUAGES.md` and the frozen contract.
