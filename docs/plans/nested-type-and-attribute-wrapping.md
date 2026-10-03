# Work item: nested type lists, attribute/decorator arguments, and Haskell/Elm type forms

Status: proposed
Base: branch `feat/canonical-line-wrapping`, commit `77bb92b`
Owner: (assign on pickup)

This document is self-contained. Read it top to bottom before writing code.

## 1. Goal

Canonical projections wrap a declaration's primary bracketed list and
Elm/Haskell arrow chains at a fixed 80-column budget (`base::LINE_WIDTH`).
Real repositories still produce long lines because four structural families are
rendered as single flat strings:

1. **Nested type lists.** A bracketed construct inside a type cannot break.
   Python subscripts (`list[...]`, `dict[..., ...]`), Rust generic
   arguments/tuples (`Result<...>`, `(&'static str, ...)`), and Haskell
   `parens`/`tuple`/`list`/`apply`/`infix` type atoms are the common cases.
2. **Attribute and decorator argument lists.** Rust attributes
   (`#[command(...)]`, `#[arg(...)]`) and Python decorators
   (`@app.get(..., response_model=...)`) are preserved as flat token strings.
3. **Haskell type synonyms.** A `type API = ... :> ... :<|> ...` right-hand
   side is emitted with `node_text`, so a Servant-style API type cannot break.
4. **Elm record and tuple types.** Records inside a type annotation
   (`view : { ... }`) and inside a variant body (`= Foo { ... }`) always use the
   flat record form.

The deliverable is that these constructs break one item per indented line when
they do not fit, with **no change to output for anything that already fits**.
Lines dominated by a single atomic token (a string literal or an identifier
longer than 80 columns) will still exceed the budget; that is unavoidable and
out of scope (see section 8).

### Acceptance signal, and its limit

Run at the repository root:

```sh
cargo build --workspace --release
./target/release/codect show --mode signatures | rg -n '.{81,}'
./target/release/codect show --mode types      | rg -n '.{81,}'
```

At `77bb92b` the signatures projection reports 19 lines and the types
projection 16. **Every one of them comes from Rust** (`crates/`), because this
repository's committed `.py`, `.hs`, and `.elm` files are small fixtures whose
projections already fit. This repository-level check therefore measures the
Rust work only; it cannot detect whether the Python, Haskell, or Elm work was
done at all.

After this work the signatures remainder must be only lines whose overflow is
inside one atomic token, and the types remainder must be the same set of
attributes. The expected Rust remainder is:

- `#[command(... long_about = "...")]` and `#[error("...")]` lines whose
  overflow is inside one string literal (several lines; some are a single
  multi-line `\`-continued literal).
- Two test function names longer than 80: `fn retained_outline_text_...`,
  `fn batched_commit_navigation_...` (no parameter list to break).

Everything else, including
`crates/engine/src/engine.rs` `snapshot_entries`'s
`Result<(&'static str, String, Vec<(RepoPath, SnapshotEntry)>), EngineError>`
return type, must be gone. Because the repository check cannot exercise
Python/Haskell/Elm, **the per-language fixture corpora in section 7 are the
real regression net for those three languages.**

## 2. Background: how rendering works today

Read these before starting:

- `crates/base/src/render.rs` — defines `pub const LINE_WIDTH: usize = 80`.
- `crates/language-{python,rust,elm,haskell}/src/render.rs` — each has its
  own `Doc` pretty-printer (duplicated by convention; do not unify).
- `crates/language-*/src/extract.rs` — builds `Doc`s from the syntax tree.

`Doc` variants (python/rust; elm/haskell have a subset):

```rust
enum Doc {
    Text(String),          // literal
    Line,                  // hard newline
    SoftLine,              // " " flat, newline+indent broken
    SoftNil,               // ""  flat, newline+indent broken
    Broken(&'static str),  // emitted only when the enclosing group is broken
    Indent(Box<Doc>),
    Group(Box<Doc>),       // flat iff column_now + flat_width <= LINE_WIDTH
    Concat(Vec<Doc>),
}
```

The renderer already supports **nested** groups: a flat parent forces children
flat; a broken parent lets each child re-decide at its own column. This work is
purely about building richer `Doc`s, not changing the renderer core.

**Prerequisite:** the Haskell and Elm `Doc` enums have only
`Text`/`Line`/`SoftLine`/`Indent`/`Group`/`Concat`. Add `SoftNil` and `Broken`
(and the matching `flat_width` / `render_into` arms) to both, because breakable
bracket lists cannot be expressed without them. This is a required code change
in `crates/language-haskell/src/render.rs` and
`crates/language-elm/src/render.rs` and must not change flat output.

Existing list builders:

- Python `crates/language-python/src/render.rs`:
  `Renderer::bracket_list(&self, node, open, close) -> Doc` builds an
  **ungrouped** list body from a node's named children, using `SoftNil` after
  the open delimiter, `"," + SoftLine` between items, `Broken(",")` after the
  last item, and `SoftNil` before the close delimiter. `extract.rs` groups the
  whole header around it.
- Rust `crates/language-rust/src/render.rs`:
  `bracket_list(items: &[String], open, close) -> Doc` (same shape) and
  `brace_list(fields: &[String]) -> Doc` (spaced inline, `SoftLine` inner pad).
  `Elem`/`elements_doc` group all but the primary (last) list; `signature_doc`
  and `container_doc` group the header with its `;`/` {` suffix.
- Python `extract.rs::decorators` renders each decorator with `node_text(child)`.
- Rust `render.rs::attribute_text` renders an attribute with `render_node`.

Trailing-comma policy (already shipped for Python/Rust, keep it): a broken list
emits a trailing comma; an inline list does not. This makes appending an item a
one-line diff. **Do not apply this policy to Haskell or Elm.** Their existing
record blocks use a leading-comma style with no trailing comma (see
`fixtures/elm/type-aliases/` and the Haskell `record_block`):

```elm
{ name : String
, email : Email
}
```

New broken Haskell/Elm lists (records, tuples, lists) must reuse that leading
comma shape so output stays consistent inside a language.

Flat-equivalence invariant: for any document that fits the budget, the rendered
bytes must not change. Every existing fixture in `fixtures/` encodes this; if
your change alters a short type's output, existing fixture tests fail. Use that
as your regression net.

## 3. Discovery step (do this first, per language)

The exact tree shapes differ by grammar version. Confirm them before coding.
Add a temporary `#[test]` that prints `root.to_sexp()` for representative
inputs, run it with `--nocapture`, then delete it. Keep the test out of the
final diff.

Python inputs to dump (`tree-sitter-python` 0.25):

```python
def f(a: list[int], b: dict[str, tuple[int, str]]) -> list["A" | "B"]: ...
@app.get("/x", response_model=Y, status_code=200)
def g(): ...
```

Rust inputs to dump (`tree-sitter-rust` 0.24):

```rust
pub fn f(v: Vec<Result<A, B>>) -> Result<(&'static str, Vec<(RepoPath, SnapshotEntry)>), EngineError>;
#[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP)]
struct S;
```

Haskell inputs to dump (`tree-sitter-haskell` 0.23):

```haskell
journalWithHistoricalCostsUsing :: (Day -> Hledger.MixedAmount -> Hledger.MixedAmount)

scopeScanFixtures :: [(String, Text.Text, AssetClassMappings, InvestmentMappings, [Maybe Text.Text])]

genSummaryScenario :: Gen (Text.Text, AssetClassMappings, InvestmentMappings, Maybe Text.Text)

type API = "api" :> "v1" :> Header "X-Request-ID" Text :> (SystemAPI :<|> AccountsAPI :<|> TransactionsAPI)
```

Elm inputs to dump (`tree-sitter-elm` 5.9):

```elm
view : { title : String, subtitle : String, healthStatus : String, isHealthy : Bool, windowWidth : Int, children : List (Element msg) }

type AccountNodeDto = AccountNodeDto { path : String, name : String, accountType : Maybe AccountType, directBalances : List Quantity }
```

Node kinds already confirmed for this work:

Python
- `subscript` — fields `value` (primary_expression), `subscript` (expression|slice).
- `binary_operator` — fields `left`, `operator` (`|` for unions), `right`.
- `decorator` — no fields; children are `@` plus an expression.
- `call` — fields `function` (primary_expression), `arguments` (argument_list).
- `typed_parameter` / `typed_default_parameter` — field `type`.

Rust
- `generic_type` — fields `type`, `type_arguments` (a `type_arguments` node that
  includes the `<` `>` tokens).
- `tuple_type` — children are element types and `,` tokens.
- `reference_type` — field `type`.
- `function_type` — fields `parameters`, `return_type`, `trait`.
- `attribute` — fields `value` (the path) and `arguments` (a `token_tree`).
- `field_declaration` — fields `name`, `type`.
- `parameter` — fields `pattern`, `type`.

Haskell
- `function` — fields `parameter`, `result` (the arrow chain).
- `context` — fields `context`, `type` (a leading `C a =>`).
- `parens` — field `type` (a parenthesized type).
- `tuple` / `unboxed_tuple` — field `element`, repeated.
- `list` — field `element`.
- `apply` — fields `constructor`, `argument`.
- `infix` — fields `left_operand`, `operator`, `right_operand`.
- `type_synomym` (note the grammar's spelling) — fields `name`, `patterns`,
  `type`.

Elm
- `record_type` — fields `fieldType` (repeated `field_type`), `baseRecord`.
- `tuple_type` — field `typeExpression`, repeated.
- `type_ref` — an `upper_case_qid` followed by `part` children.
- `type_expression` — arrow chain (already handled).
- `union_variant` — fields `name`, `part` (constructor arguments).

The `token_tree` of a Rust attribute is an opaque token tree; split it at
depth-1 commas by walking its children and counting `(`/`[`/`{` versus
`)`/`]`/`}`, splitting on `,` when the depth returns to 1.

## 4. Part A — nested type lists

### 4.1 Shared refactor: list items become `Doc`s

So nested lists can nest, change both list builders to take `Vec<Doc>`:

- Rust: `fn bracket_list(items: &[Doc], open: &str, close: &str) -> Doc` and
  `fn brace_list(fields: &[Doc]) -> Doc`. Update `Elem::List` to hold
  `items: Vec<Doc>` and `list_elem` to push `Doc::Text(render_node(child))`.
- Python: `Renderer::bracket_list(&self, items: Vec<Doc>, open, close) -> Doc`,
  plus a thin `node_bracket_list(&self, node, open, close)` that maps named
  children through `node_text` and calls it (keeps current call sites a
  one-line change).

Flat output is unchanged because `Doc::Text` renders identically.

### 4.2 A per-language recursive `type_doc`

Add a recursive `type_doc(node) -> Doc` to each renderer. The rule that keeps
this safe: **handle only the node kinds listed below; return
`Doc::Text(node_text(node))` / `Doc::Text(render_node(node, source))` for every
other kind.** Unknown constructs stay flat and cannot regress.

Rust `type_doc(node, source) -> Doc`:

- `generic_type`: `Doc::Concat([Text(render_node(type_field)), bracket_list(<items>, "<", ">")])`
  where `<items>` are the `type_arguments` children split on top-level commas,
  each rendered with `type_doc`. Flat: `Foo<A, B>`.
- `tuple_type`: `bracket_list(<element types>, "(", ")")`. Flat: `(A, B)`.
- `reference_type`: prefix tokens (`&`, optional lifetime, `mut`) joined flat,
  then `type_doc(type_field)`, with a single space inserted by the existing
  `needs_space` between the last prefix token and the inner type's first token.
- `function_type`: `Text("Fn")` + `bracket_list(parameters)` + optional
  `Text(" -> ")` + `type_doc(return_type)`.
- fallback: `Doc::Text(render_node(node, source))`.

Python `type_doc(node) -> Doc`:

- `subscript`: `Doc::Concat([Text(node_text(value)), subscript_slice_doc(slice)])`.
  `subscript_slice_doc` renders `[` + `Indent(SoftNil + items...)` + `SoftNil` +
  `]` using the shared bracket body. If the slice has top-level commas (an
  `expression_list`, or a node whose named children are separated by top-level
  commas), each item is a `type_doc`; otherwise the whole slice is one
  `type_doc`.
- `binary_operator` with operator `|` (union): flatten the right-leaning chain
  into atoms and emit `Group(Concat([first, Indent(Concat([SoftLine, Text("| "), atom]...))]))`,
  breaking before each `|`. Flat: `A | B | C`.
- fallback: `Doc::Text(node_text(node))`.

Haskell `type_doc(node) -> Doc`:

- `function`: keep the existing arrow chain, but render each `parameter`/`result`
  with `type_doc` instead of `node_text`, and keep the `context`/`=>` handling.
- `parens`: `Group(Concat([Text("("), Indent(Concat([SoftNil, type_doc(type)])), SoftNil, Text(")")]))`.
  Flat: `(A -> B)`.
- `tuple` / `unboxed_tuple`: bracket body over the `element` children, one per
  line when broken, **leading-comma style** (`( a\n, b\n)`), no trailing comma.
- `list`: `[` + `Indent(SoftNil + type_doc(element))` + `SoftNil` + `]`.
- `apply`: `Doc::Concat([type_doc(constructor), Text(" "), type_doc(argument)])`.
  Flat: `Gen (A, B)`.
- `infix`: flatten the chain into operands and break **before** each operator,
  `SoftLine + Text(operator + " ") + operand`, matching the arrow layout.
  Flat: `"api" :> "v1"`.
- fallback: `Doc::Text(node_text(node))`.

Elm `type_doc` (extend the existing `type_atom`/`type_expression`):

- `record_type`: build a breakable record with the existing leading-comma block
  shape. Flat `{ a : A, b : B }`; broken
  `{ a : A\n, b : B\n}`. The `field_type` values are themselves `Doc`s. Keep
  the base-record form `{ base | a : A }`.
- `tuple_type`: bracket body over the `typeExpression` children, leading-comma
  style. Flat `(String, Int)`.
- `type_ref`: keep the space-joined form; each argument is a `type_doc` so a
  nested record/tuple can break. (Optional: allow `SoftLine` between arguments.)
- `record_type` and `tuple_type` must be reachable from `type_annotation`,
  `variant_body`, and `type_ref` arguments.

### 4.3 Wiring, in phases

Do these as separate commits/tasks if possible; each is independently useful
and testable. Keep the primary-list mechanism intact in Python and Rust (the
last list in a header stays ungrouped so the header's fit check sees the return
type).

1. **Rust return types.** In `render.rs::header_elements`, when a child is the
   function's return type (compare byte ranges with
   `node.child_by_field_name("return_type")`), push a new `Elem::Type(type_doc(child))`
   instead of its flat tokens. Boundary spacing: before the element, emit a
   space iff `needs_space(previous_token, first_token_of(child))`. `elements_doc`
   must handle `Elem::Type` by pushing the `Doc` and setting `previous` to the
   type's last token. Because the return type is now a nested `Group`, it
   re-decides after the parameter list breaks.
2. **Rust field, variant-field, alias, associated-type, and const/static
   types.** Add `field_doc(node, source) -> Doc` = `Text(format!("{name}: "))` +
   `type_doc(type_field)`. Change `field_texts` to return `Vec<Doc>` (rename to
   `field_docs`), use it from `build_field` and `variant_doc`. Use `type_doc`
   for the right-hand side of `type_alias_text`, the bounds of
   `associated_type_text`, and the declared type of `constant_text`.
3. **Python return types.** In `extract.rs::function_decl`, replace
   `Doc::Text(self.renderer.node_text(return_type))` with
   `self.renderer.type_doc(return_type)`.
4. **Python parameter types and annotations.** `bracket_list` items should
   render each parameter with `parameter_doc` (name + `: ` + optional default +
   `type_doc(type)`). Use `type_doc` for annotated assignments
   (`left: type`) and for a `type` statement's right-hand side.
5. **Haskell signatures and synonyms.** Route `signature_doc` through the new
   `type_doc`. For a signature whose type is **not** a `function` or `context`
   (a single `parens`/`list`/`tuple`/`apply`/`infix` atom), emit
   `Text("name ::")` + `Indent(Concat([SoftLine, type_doc]))` so the atom can
   move to its own indented line. This preserves every existing arrow fixture
   (they are `function`/`context`) and fixes single long atoms. Add a
   `type_synonym_doc` used by `simple_decl` for `TYPE_SYNONYM`:
   `Text("type Name params =")` + `Group(type_doc(rhs))`.
6. **Elm records and tuples.** Make `record_type` width-sensitive as in 4.2 and
   route `type_annotation`, `variant_body`, and `type_ref` through it. The
   existing `type_alias` sole-record block keeps its current hard-line form.
7. **Parameter types.** Rust `list_elem` and Python `bracket_list` items render
   each parameter with its `type_doc` so a long parameter type can break inside
   the already-broken parameter list.
8. **Optional.** Rust `where` predicates, Haskell class/instance headers, and
   Haskell non-record data-constructor argument lists are single flat lines
   today. Only extend them if the acceptance check (section 10) still shows a
   structural overflow there.

## 5. Part B — attribute and decorator argument lists

Reuse the list-group shape; each item is a flat `Doc::Text` (argument token
streams are not recursive here).

Rust (`render.rs`):

- Replace `attribute_text -> String` with `attribute_doc(node, source) -> Doc`
  used by `attribute_docs` (in `extract.rs`).
- If the attribute has an `arguments` `token_tree` whose contents include a
  top-level comma, emit
  `Group(Concat([Text("#["), Text(path_text), arg_list_doc, Text("]")]))`
  where `arg_list_doc` is the bracket body over the depth-1 comma-split items
  (each item = `join(tokens)` of that item's leaves). Otherwise fall back to
  `Doc::Text(render_node(attribute, source))`.
- `#[` attaches to the path and `]` attaches to `)` with no spaces, so build
  those as literal text rather than through `needs_space`.
- Flat equivalence: `#[arg(long = "path", short = 'p')]` must be byte-identical.

Python (`extract.rs::decorators`):

- Add `Renderer::decorator_doc(node) -> Doc`. For a decorator whose expression is
  a `call`, emit
  `Group(Concat([Text("@"), Text(node_text(call.function)), bracket_list(<arg items>, "(", ")")]))`.
  Otherwise `Doc::Text(node_text(decorator))`.
- `argument_list` named children are the items.

Trailing comma applies here too for Python/Rust (broken list ⇒ trailing comma).
Attribute and decorator token trees are whitespace-insensitive between tokens,
so inserting newlines is safe. Never break inside a string literal.

## 6. Expected outputs (make these pass)

Python nested return (no parameter list to absorb the overflow):

```python
def f() -> dict[str, tuple[AutochargeInsuranceRemitsCron, PatientArAutochargeCron, OrganizationBillingProfile]]: ...
```

```python
def f() -> dict[
    str,
    tuple[
        AutochargeInsuranceRemitsCron,
        PatientArAutochargeCron,
        OrganizationBillingProfile,
    ],
]: ...
```

Rust nested return (parameter list breaks first, then the return type):

```rust
    pub fn snapshot_entries(&self, spec: &str) -> Result<(&'static str, String, Vec<(RepoPath, SnapshotEntry)>), EngineError>;
```

```rust
    pub fn snapshot_entries(
        &self,
        spec: &str,
    ) -> Result<
        (&'static str, String, Vec<(RepoPath, SnapshotEntry)>),
        EngineError,
    >;
```

Rust multi-arg attribute:

```rust
    #[command(after_help = FOCUSED_DIFF_HELP, after_long_help = FOCUSED_DIFF_HELP)]
```

```rust
    #[command(
        after_help = FOCUSED_DIFF_HELP,
        after_long_help = FOCUSED_DIFF_HELP,
    )]
```

Python decorator:

```python
@app.get("/very/long/path/here", response_model=VeryLongResponseModel, status_code=200)
def handler(): ...
```

```python
@app.get(
    "/very/long/path/here",
    response_model=VeryLongResponseModel,
    status_code=200,
)
def handler(): ...
```

Haskell parenthesized arrow atom (breaks after `::`, inner chain re-decides):

```haskell
journalWithHistoricalCostsUsing ::
    (Day -> Hledger.MixedAmount -> Hledger.MixedAmount)
```

Haskell type application with a tuple argument:

```haskell
genSummaryScenario ::
    Gen (Text.Text, AssetClassMappings, InvestmentMappings, Maybe Text.Text)
```

Haskell `list` of a long `tuple` (exact nesting is fixed by the fixture; the
tuple breaks inside the broken list):

```haskell
scopeScanFixtures ::
    [
        (
            String,
            Text.Text,
            AssetClassMappings,
            InvestmentMappings,
            [Maybe Text.Text],
        ),
    ]
```

Haskell type synonym with an operator chain (operators lead their lines; a
nested `parens`/`apply`/`tuple` re-decides at its own column and stays flat when
it fits there, so only the outer chain breaks here):

```haskell
type API =
    "api"
        :> "v1"
        :> Header "X-Request-ID" Text
        :> (SystemAPI :<|> AccountsAPI :<|> TransactionsAPI)
```

Elm record annotation (leading commas, no trailing comma; the first field
stays on the `name :` line):

```elm
view : { title : String
    , subtitle : String
    , healthStatus : String
    , isHealthy : Bool
    , windowWidth : Int
    , children : List (Element msg)
    }
```

An item that still exceeds the budget after breaking (a long single string or
identifier) must stay on its own long line rather than be split.

## 7. Test plan (TDD: write fixtures before implementation)

Fixtures are auto-discovered per language; each case is a directory with an
input and a `types.txt` / `signatures.txt`. See `fixtures/*/line-wrapping/` and
`crates/language-*/tests/fixtures.rs`. `codect show` scans `fixtures/`, so
a fixture whose projection is still long will show up in the section-10 check;
keep each fixture's expected projection wrapped.

Add cases:

- `fixtures/python/nested-types/` — a long subscript return type and a long
  decorator; `types.txt` empty, `signatures.txt` as in section 6.
- `fixtures/rust/nested-return/` — `snapshot_entries`-like function; assert the
  nested `Result<...>` wrap in both modes (`types.txt` empty if no types).
- `fixtures/rust/attribute-args/` — a struct with a long multi-arg attribute;
  both modes include the attribute.
- `fixtures/haskell/type-synonyms/` — a `type API = ... :> ... :<|> ...` and a
  long `Gen (...)`/`[...]` signature.
- `fixtures/elm/record-wrapping/` — a long record annotation and a record
  variant body; both modes.
- Optionally extend `fixtures/{python,rust,haskell,elm}/line-wrapping/` rather
  than adding new directories; keep each case focused.

Unit tests:

- Rust `render.rs`: `type_doc` for `Vec<A, B>`, `(A, B)`, `&'a Vec<A>` breaks at
  nested levels; flat output equals `render_node` for short types.
- Python `render.rs`: `type_doc` for `list[A | B]`, `dict[str, int]`,
  `tuple[A, B]`; flat output equals `node_text` for short types.
- Haskell `render.rs`: `type_doc` for `(A -> B)`, `[A]`, `(A, B)`, `Gen (A, B)`,
  `A :> B`; flat output equals `node_text` for short types.
- Elm `render.rs`: `record_type` and `tuple_type` flat equality for short
  cases and the leading-comma broken shape for long ones.
- `attribute_doc` / `decorator_doc`: flat equality for a short case and a
  broken shape for a long one, including the trailing comma.
- Boundary: a construct whose flat form is exactly 80 stays inline; 81 breaks.

Regression: the entire existing `fixtures/` corpus must still pass unchanged
(except the handful of fixtures that legitimately cross 80 and must be updated
deliberately — keep that list small and review each).

End-to-end: extend `crates/cli/tests/cli.rs` with one `show` test that
asserts a nested-type projection, following
`show_wraps_a_long_python_signature_within_the_line_budget`.

## 8. Non-goals and irreducible cases

- **Atomic tokens are never split.** A line whose overflow is inside one string
  literal, character literal, or identifier stays long. This includes
  `#[error("...")]`, `#[command(long_about = "...")]`, and >80-character
  function names. `rustfmt` behaves the same way. Do not attempt to chop a
  string or identifier.
- No terminal-width dependence. The budget stays the compile-time
  `LINE_WIDTH`.
- No source-layout dependence: the decision is the construct's flat width, not
  where the author put newlines.
- Do not unify the per-crate `Doc` types.
- Rust macro token trees other than attribute argument lists (e.g.
  `macro_rules!` bodies) are not rendered and are out of scope.

## 9. Files expected to change

- `crates/language-rust/src/render.rs`, `src/extract.rs`
- `crates/language-python/src/render.rs`, `src/extract.rs`
- `crates/language-haskell/src/render.rs`, `src/extract.rs`
- `crates/language-elm/src/render.rs`
- New fixtures under `fixtures/{python,rust,haskell,elm}/`
- `crates/cli/tests/cli.rs` (one end-to-end test)
- `docs/TECHNICAL_DESIGN.md` sections 10 (Rust/Python) and 11 (Elm/Haskell):
  add nested type lists, attribute/decorator argument lists, Haskell type
  synonyms, and Elm record/tuple breaking to the rules, and keep the
  atomic-token caveat.

## 10. Verification

```sh
direnv exec . cargo fmt --all
direnv exec . cargo test --workspace
direnv exec . cargo clippy -p base -p language-python -p language-rust \
  -p language-haskell -p language-elm --all-targets -- -D warnings
direnv exec . cargo build --workspace --release
./target/release/codect show --mode signatures | rg -n '.{81,}'
./target/release/codect show --mode types      | rg -n '.{81,}'
```

Note: `cargo clippy --workspace --all-targets -- -D warnings` (what
`just check` runs) currently fails on a **pre-existing** `clippy::type_complexity`
lint in `crates/engine/src/engine.rs:559` under clippy 1.98, on `main` as
well. That is unrelated to this work; do not fix it here unless asked.

Measure both `show` commands before and after; the Rust remainder must drop to
the atomic-only set described in section 1. The Haskell/Elm/Python fixtures are
the evidence for those languages.

## 11. Risks

- Type-node shapes vary by grammar version; confirm every assumption with a
  temporary tree dump (section 3) rather than guessing.
- Flat-output drift is the main correctness risk, now across all four
  languages. Adding `SoftNil`/`Broken` to the Haskell/Elm `Doc` enums must not
  change any flat render; the existing fixture suite is the guard. Run it after
  every task and update fixtures only for cases that intentionally cross 80.
- Style divergence: Python/Rust use a trailing comma in broken lists; Haskell
  and Elm use leading commas with no trailing comma. Mixing them is a bug.
- Over-breaking is a UX risk: a type that fits must stay inline. The nested-group
  renderer handles this, but verify with boundary tests at 80/81 columns.
- Rust trailing commas inside generic arguments and tuple types are valid; verify
  the single-element Python subscript/tuple cases do not change meaning.
- The repository-level acceptance check exercises Rust only. Do not treat a
  clean signatures/types run as evidence that Python/Haskell/Elm are correct;
  rely on section 7.

## 12. Work breakdown (execution order, smallest first)

Executed by subagents with an orchestrator reviewing each result. Each task
builds and tests its own crate before the next starts.

1. **Rust attribute argument lists** (Part B Rust). `render.rs::attribute_doc`,
   `extract.rs::attribute_docs`, `fixtures/rust/attribute-args/`. Smallest.
2. **Python decorator argument lists** (Part B Python).
   `Renderer::decorator_doc`, `extract.rs::decorators`, Python fixture.
3. **Elm record and tuple types** (Part A Elm; adds `SoftNil` to the Elm `Doc`).
   `render.rs::record_type`/`tuple_type`, `fixtures/elm/record-wrapping/`.
4. **Haskell type synonyms** (Part A Haskell). `type_synonym_doc` over the new
   `type_doc`'s `infix` handling; `fixtures/haskell/type-synonyms/`.
5. **Haskell nested type atoms** (Part A Haskell; adds `SoftNil`/`Broken` to the
   Haskell `Doc`). `parens`/`tuple`/`list`/`apply`/`infix` and the
   signature-after-`::` fallback.
6. **Rust nested type lists** (Part A Rust). `bracket_list` over `Vec<Doc>`,
   `Elem::Type`, `type_doc`, field/alias/const wiring,
   `fixtures/rust/nested-return/`.
7. **Python nested type lists** (Part A Python). `bracket_list` over `Vec<Doc>`,
   `type_doc`, return/parameter/annotation wiring,
   `fixtures/python/nested-types/`.
8. **Finalize.** §10 verification in both modes, the `cli.rs` end-to-end test,
   and the `docs/TECHNICAL_DESIGN.md` sections 10/11 updates.
