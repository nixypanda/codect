# OwnAI MVP Technical Design

## 1. Purpose and authority

This document specifies how to implement the OwnAI MVP. It is intended to be detailed enough for an implementation agent to work from without inventing architecture or product behavior.

[PRODUCT.md](./PRODUCT.md) is authoritative for product behavior and scope. If this document conflicts with the product document, follow the product document and update this document in the same change.

The MVP supports:

- Elm and Rust source files.
- Types and Signatures projection modes.
- Showing a projection for a Git commit.
- Diffing projections from two Git commits.

The MVP does not perform type inference, expand macros, inspect function bodies, read the index or working tree, or implement Public and Full modes.

## 2. Technical decisions

| Concern | Decision |
|---|---|
| Implementation language | Rust, stable toolchain, Rust 2024 edition |
| Project structure | Cargo workspace with separate core, language, Git, and CLI crates |
| Git access | `gix`, read-only, behind an OwnAI-owned interface |
| Parsing | Tree-sitter with the Elm and Rust grammars |
| Projection | Language-specific extraction into a small shared projection model |
| Rendering | Deterministic, language-specific canonical rendering |
| Diffing | Line-oriented patience diff over canonical projected text |
| CLI parsing | `clap` derive API |
| Errors | Typed library errors with `thiserror`; user-facing reports with `miette` |
| Testing | Fixture and snapshot tests, plus end-to-end CLI tests over temporary Git repositories |

The engine and CLI are compiled into one executable. There is no daemon, IPC protocol, plugin host, LSP client, or MCP server in the MVP.

## 3. Workspace layout

Create this workspace:

```text
Cargo.toml
Cargo.lock
crates/
  ownai-core/
    src/
      lib.rs
      diagnostic.rs
      diff.rs
      language.rs
      model.rs
      render.rs
  ownai-git/
    src/
      lib.rs
      repository.rs
      revision.rs
      tree.rs
  ownai-language-elm/
    src/
      lib.rs
      extract.rs
      render.rs
      syntax.rs
  ownai-language-rust/
    src/
      lib.rs
      extract.rs
      render.rs
      syntax.rs
  ownai-cli/
    src/
      main.rs
      args.rs
      command.rs
      output.rs
fixtures/
  elm/
  rust/
tests/
```

Dependency direction is one-way:

```text
ownai-cli
  ├── ownai-core
  ├── ownai-git
  ├── ownai-language-elm
  └── ownai-language-rust

ownai-language-elm  ──→ ownai-core
ownai-language-rust ──→ ownai-core
ownai-git           ──→ ownai-core
```

`ownai-core` must not depend on `gix`, Tree-sitter, either grammar, or `clap`. Language-specific Tree-sitter node names must never appear in `ownai-core`. `gix` types must never leave `ownai-git`.

## 4. Dependencies

Declare shared dependency versions under `[workspace.dependencies]`. Initially use compatible releases in these major/minor families and commit `Cargo.lock`:

```toml
[workspace.dependencies]
tree-sitter = "0.27"
tree-sitter-elm = "5.9"
tree-sitter-rust = "0.24"
gix = { version = "0.87", default-features = false, features = [
  "auto-chain-error",
  "revision",
  "sha1",
  "sha256",
  "pack-cache-lru-static",
] }
similar = "3.2"
clap = { version = "4", features = ["derive"] }
thiserror = "2"
miette = { version = "7", features = ["fancy"] }
bstr = "1"
anstream = "1.0"
anstyle = "1"
```

Use current compatible releases for test-only dependencies:

- `insta` for projection and diff snapshots.
- `assert_cmd` for CLI tests.
- `predicates` for CLI assertions.
- `tempfile` for temporary repositories.

Do not add an async runtime. All MVP work is local and synchronous.

### 4.1 `gix` features

Keep `default-features = false`. Enable only:

- `revision`: revision parsing, peeling, and merge-base support. This currently brings `index` transitively; do not separately use the index in the MVP.
- `sha1`: normal Git object IDs.
- `sha256`: SHA-256 repositories.
- `auto-chain-error`: useful error sources for CLI diagnostics.
- `pack-cache-lru-static`: efficient reads from packed object databases without enabling the larger performance bundle.

Do not enable these feature groups in the MVP:

- `basic` or any default bundle.
- `blob-diff`; OwnAI diffs projections, not Git blobs.
- `attributes`, `excludes`, or `dirwalk`.
- `status`, `worktree-stream`, `worktree-archive`, or `worktree-mutation`.
- `merge`, `blame`, `mailmap`, or `notes`.
- `credentials` or any network client or transport.
- `parallel`, `max-control`, or `max-performance` until profiling justifies them.
- `serde`; no `gix` value is serialized outside `ownai-git`.
- `revparse-regex`; the MVP does not promise regex commit-message revision searches.

Review this list whenever `gix` is upgraded because it is pre-1.0 and feature relationships may change.

### 4.2 Tree-sitter features

Use the native Tree-sitter runtime with its default `std` support. Do not enable Tree-sitter's WASM runtime. Load the grammars through `tree_sitter_elm::LANGUAGE` and `tree_sitter_rust::LANGUAGE`.

At startup in tests, assert that both grammars can be assigned to a parser. Grammar upgrades must include a review of `NODE_TYPES` and all affected snapshots.

## 5. Core model

The shared model describes a projection, not a universal programming-language AST.

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionMode {
    Types,
    Signatures,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    Elm,
    Rust,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ItemKind {
    Module,
    Type,
    TypeAlias,
    Constructor,
    Field,
    Variant,
    Trait,
    TraitImplementation,
    AssociatedType,
    Function,
    Method,
    Value,
    Constant,
    Static,
    Port,
    Operator,
    ForeignBlock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedItem {
    pub stable_key: String,
    pub parent_key: Option<String>,
    pub kind: ItemKind,
    pub name: String,
    pub span: SourceSpan,
    pub canonical_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedFile {
    pub path: RepoPath,
    pub language: Language,
    pub items: Vec<ProjectedItem>,
    pub canonical_text: String,
}
```

The exact enum can grow while implementing fixtures, but do not store language AST nodes, Tree-sitter nodes, or `gix` handles in it.

`ProjectedFile::canonical_text` must be derived from `items` by a constructor and must not be independently mutable. Keep fields private where doing so enforces this invariant. `SourceSpan` line and column values are zero-based; CLI diagnostics may convert them to one-based display values.

### 5.1 Repository paths

Git paths are byte strings, not guaranteed UTF-8 operating-system paths. `RepoPath` must own normalized repository-relative bytes and use `/` as the separator. Base it on `bstr::BString` or an equivalent owned byte representation.

Requirements:

- Reject absolute paths and `..` traversal.
- Sort by raw path bytes for deterministic output.
- Detect `.elm` and `.rs` using ASCII extension bytes.
- Escape invalid UTF-8 when displaying a path.
- Do not convert a repository path to `PathBuf` merely to inspect a committed tree.

Elm and Rust source contents must be valid UTF-8. A supported source blob containing invalid UTF-8 is a fatal projection diagnostic.

### 5.2 Stable keys

Stable keys identify declarations inside a projected file. They are not global database IDs and are not displayed by default.

- Elm module: declared module name.
- Elm type/value: module name, item kind, and declared name.
- Rust file module: repository path.
- Rust inline module: parent key plus module name.
- Rust free item: container, item kind, and name.
- Rust inherent implementation: normalized target type.
- Rust trait implementation: normalized trait path plus normalized target type.
- Rust associated item: implementation or trait key plus item kind and name.

If two declarations produce the same key, append a deterministic source-order ordinal. Never include byte offsets in the primary key because harmless edits before a declaration would destabilize it.

### 5.3 Canonical text assembly

`ProjectedFile::canonical_text` is derived from `items` by the constructor and is never independently mutable. Emission is determined solely by an item's `parent_key`:

1. Only items whose `parent_key` is `None` are emitted. Their `canonical_text` fragments are joined by exactly one blank line, and the result ends with exactly one trailing newline. A file with no top-level items has empty canonical text.
2. An item whose `parent_key` is `Some(_)` is an index-only entry. Its text is already contained in the fragment of the ancestor that owns it. It is never emitted as its own top-level block.
3. Every item's `canonical_text` is a self-contained fragment. A top-level fragment includes the canonical rendering of its nested members, indented four spaces per nesting level (section 10). A nested item's text appears exactly once in the file text, inside its ancestor.
4. The `parent_key` relationship is independent of the container naming used in stable keys (section 5.2). Naming a container in a stable key does not suppress emission; only `parent_key == Some(_)` does. An adapter marks each declaration it wants emitted as top-level.
5. Every nested item must have a top-level ancestor, and an adapter must not produce an item whose `parent_key` refers to a non-existent item. This is a documented adapter obligation; core does not validate it at runtime.

Nested items remain in `items` so that stable keys and later per-declaration comparisons can address them, even though their text is emitted through an ancestor.

## 6. Language adapter interface

Expose this conceptual interface from `ownai-core`:

```rust
pub struct ProjectionInput<'a> {
    pub path: &'a RepoPath,
    pub source: &'a str,
    pub mode: ProjectionMode,
}

pub trait LanguageProjector: Send + Sync {
    fn language(&self) -> Language;
    fn supports_path(&self, path: &RepoPath) -> bool;
    fn project(&self, input: ProjectionInput<'_>)
        -> Result<ProjectedFile, ProjectionError>;
}
```

The adapters own all language meaning. Core selects an adapter by path, invokes it, renders project/file framing, and compares canonical text.

Parsing and projection must be deterministic and must not depend on the current directory, locale, terminal width, wall clock, environment variables, or installed compilers.

## 7. End-to-end pipeline

### 7.1 Show

```text
repository path + revision
  → discover repository
  → resolve revision to one commit
  → traverse commit tree
  → select .elm and .rs blobs
  → read each blob
  → project with its language adapter
  → sort files by raw repository path
  → render file sections
  → write output
```

### 7.2 Diff

```text
base revision + target revision
  → resolve both to commits
  → traverse both trees
  → union supported paths
  → for each path in byte order:
      unchanged blob ID → skip
      old blob only     → project old; compare with empty
      new blob only     → compare empty with projected new
      both blobs        → project both; compare projections
  → emit only projected files with differences
```

A source blob change that produces the same projection produces no output. This is the defining invariant of focused diffing.

## 8. Git layer using `gix`

`ownai-git` is read-only. It owns repository discovery, revision resolution, commit peeling, tree traversal, and blob reads.

Expose OwnAI-owned values:

```rust
pub struct Revision {
    pub object_id: ObjectId,
}

pub struct ObjectId {
    pub kind: HashKind,
    pub bytes: Vec<u8>,
}

pub enum HashKind {
    Sha1,
    Sha256,
}

pub struct SourceEntry {
    pub path: RepoPath,
    pub blob_id: ObjectId,
    pub executable: bool,
}

pub trait SnapshotRepository {
    fn resolve_commit(&self, spec: &str) -> Result<Revision, GitError>;
    fn source_entries(&self, revision: &Revision)
        -> Result<Vec<SourceEntry>, GitError>;
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;
}
```

The concrete implementation wraps `gix::Repository`, but callers must not see that type.

### 8.1 Repository discovery

- Begin discovery at the CLI's current directory.
- Support normal repositories, bare repositories, and linked worktrees when `gix` can discover them.
- Open the repository read-only.
- Respect `gix` repository trust handling; do not override trust to force-load unsafe configuration.
- Return a clear diagnostic when no repository is found.

### 8.2 Revision resolution

Use `gix` revision parsing and require the result to identify one object. Peel annotated tags and other commit-ish objects to a commit.

MVP-supported forms:

- Full and unambiguous abbreviated object IDs.
- `HEAD`.
- Local branch names.
- Tag names.
- Parent and ancestor suffixes such as `HEAD^` and `HEAD~3` when supported by `gix` revision parsing.

Reject ranges such as `A..B` and `A...B` when passed as one argument. The `diff` command takes two independent revision arguments. Reject a tree or blob that cannot peel to a commit.

### 8.3 Tree traversal

- Recursively visit each commit tree.
- Retain regular and executable blob entries ending in `.elm` or `.rs`.
- Ignore directories after descending into them.
- Ignore symlinks, Git links/submodules, and unsupported file types.
- Do not perform rename detection.
- Do not read `.gitignore`; committed trees are authoritative.
- Return entries sorted by raw repository path bytes.

Use object IDs to avoid reading or projecting unchanged blobs during diff.

### 8.4 Object caching

Configure a bounded `gix` object cache suitable for repeated tree and blob access. Keep cache sizing in `ownai-git` and use a conservative constant initially. Do not expose tuning flags in the MVP. Add benchmarks before changing cache strategy or enabling broader `gix` performance features.

OwnAI needs no persistent projection cache in the MVP.

## 9. Tree-sitter integration

Each language crate owns a parser constructor and all node-kind knowledge for its grammar.

Rules:

- Create a fresh parser per projection operation initially. Pooling is unnecessary until profiling shows otherwise.
- Parse UTF-8 source bytes in one operation; incremental parsing is not needed for commit snapshots.
- Treat a `None` parse result as fatal.
- Treat a root containing `ERROR` or missing nodes as a fatal projection error.
- Include the first useful error range in the diagnostic.
- Never guess around erroneous syntax or silently emit a partial file.
- Continue collecting errors from other files internally, but do not print a focused result that could be mistaken for complete when any supported file failed.

Prefer explicit tree traversal and named field access for hierarchical declarations. Tree-sitter queries may be used for simple captures, but do not make a single large query the adapter's entire data model.

Centralize node-kind string constants in each crate's `syntax.rs`. Grammar upgrades should fail focused tests when node names or shapes change.

## 10. Canonical rendering

Canonical rendering prevents formatting-only source edits from appearing in focused diffs. Source slices alone are not acceptable output.

Rendering must be independent of original whitespace and comments. It must preserve semantic token order, declaration order, member order, visibility, generic parameters, constraints, and modifiers selected by the product rules.

Do not implement adaptive line wrapping. Terminal width must never change output. Use fixed structural line breaks:

- One simple declaration or signature per line.
- One union constructor, enum variant, struct field, or record field per indented line when a declaration has a body.
- One trait or implementation member per indented block.
- Blank line between top-level projected declarations (see section 5.3 for how top-level items assemble into file text).
- Four spaces per nesting level.
- Exactly one trailing newline per projected file.

Build a small internal document representation such as `Text`, `Space`, `Line`, `Indent`, and `Concat`, or equivalent direct rendering helpers. Do not add a complete source formatter.

For type expressions and signature fragments, walk leaf tokens while excluding comments. Normalize spacing with language-specific rules. At minimum:

- Prevent adjacent identifiers/keywords/literals from merging.
- No spaces immediately inside `()`, `[]`, or generic angle brackets.
- No spaces before commas or semicolons.
- One space after commas when content follows on the same line.
- Preserve and normalize Elm `:`, `=`, `|`, and `->` spacing.
- Preserve and normalize Rust `:`, `=`, `->`, `+`, `where`, `for`, `as`, `::`, references, lifetimes, and ABI strings.

Every renderer rule requires a fixture showing that equivalent whitespace and comment arrangements produce identical output.

### 10.1 File framing

The canonical text stored in `ProjectedFile` contains only that language's projection. Project-level framing belongs to the core renderer.

For `show`, use an unambiguous neutral header:

```text
== src/User.elm ==
<canonical projection>
```

For `diff`, use familiar per-file headers:

```text
diff --ownai a/src/User.elm b/src/User.elm
--- a/src/User.elm
+++ b/src/User.elm
```

Use `/dev/null` for the absent side of an added or deleted file. Diff headers use escaped display paths when raw paths are not UTF-8.

Multi-file document policy (pinned so it cannot drift):

- `show` sections are separated by exactly one blank line.
- `diff` blocks are concatenated with no blank line between them, and only files whose projections differ are emitted.
- A non-empty document ends with exactly one trailing newline; a document with no emitted files is empty.
- The `diff --ownai` line keeps `a/` and `b/` labels even for an added or deleted file, while the absent `---`/`+++` side uses `/dev/null`.

## 11. Elm projection

### 11.1 General rules

- Parse the declared module name and retain whether it is a normal or port module.
- Omit the exposing clause in Types and Signatures mode because visibility filtering is not part of the MVP.
- Omit imports, comments, documentation, bodies, and declarations nested inside expressions.
- Preserve top-level declaration order after omitted declarations are removed.

### 11.2 Types mode

Include:

- `type` declarations with type parameters and every constructor.
- Constructor argument types.
- `type alias` declarations with type parameters and the complete aliased type.
- Record alias fields in source order.

A record renders in block form, one field per indented line, when it is the
complete right-hand side of a `type alias`. A record nested inside another type
expression renders inline as `{ name : Type, ... }`.

Render examples:

```elm
module User

type User
    = User UserId Profile

type alias Profile =
    { name : String
    , email : Email
    }
```

### 11.3 Signatures mode

Include everything from Types mode plus:

- Every top-level function and value definition.
- Its matching explicit type annotation when one exists.
- A visible placeholder when no annotation exists.
- Port annotations.
- Infix declarations associated with operators.

Pair annotations and definitions by their declared lower-case name, not merely adjacency. Render functions and values in definition source order. Render a standalone annotation only when it is a port; otherwise a syntactically valid Elm file should provide its matching definition.

Canonical examples:

```elm
create : UserId -> Profile -> User
```

```elm
normalize : <missing type annotation>
```

Do not inspect a function's argument patterns or body to synthesize an interface. The placeholder is deliberately non-inferential.

## 12. Rust projection

### 12.1 General rules

- Use the repository-relative file path as the outer file context.
- Preserve inline module nesting.
- Do not resolve `mod name;` to another file through Cargo module rules. The referenced file is projected independently by path.
- Omit `use`, `extern crate`, comments, documentation, function bodies, closures, local items, macro definitions, and macro invocation output.
- Preserve visibility modifiers on displayed declarations and members.
- Preserve non-documentation outer attributes attached to displayed declarations. Omit doc comments and `#[doc = ...]` attributes.
- Do not expand declarative, procedural, derive, or attribute macros.

Attributes are retained because attributes such as `repr`, `cfg`, and `non_exhaustive` can change a declaration's meaning. Retaining an attribute does not imply expanding it.

### 12.2 Types mode

Include:

- Named, tuple, and unit structs with all fields.
- Enums with all variants and variant fields.
- Unions with all fields.
- Type aliases and their right-hand types.
- Trait headers, generic parameters, supertraits, associated types, and associated type constraints/defaults.
- Trait implementation headers and associated type assignments.
- Generic parameters, const generics, lifetimes, bounds, and `where` clauses belonging to included items.
- Inline module framing needed to retain container context.

Exclude trait methods, associated functions, constants, and statics from Types mode. Exclude inherent `impl` blocks that contain no included type information.

Canonical examples:

```rust
pub struct User<T>
where T: Identity
{
    pub id: T,
    profile: Profile,
}

pub enum Status {
    Active,
    Suspended { reason: String },
}

pub trait Repository<T>: Send + Sync {
    type Error: std::error::Error;
}

impl Iterator for Users {
    type Item = User<Id>;
}
```

### 12.3 Signatures mode

Include everything from Types mode plus:

- Free functions.
- Inherent methods and associated functions.
- Trait method declarations, including methods with default bodies.
- Methods and associated functions in trait implementations.
- Foreign function declarations inside `extern` blocks.
- Associated and module-level constants and statics with declared types but without initializers.
- `async`, `const`, `unsafe`, `extern`, ABI, generic, receiver, parameter, return-type, and `where` syntax belonging to signatures.
- Implementation block headers required to identify method containers.

Replace every Rust function or method body with `;`. Remove constant and static initializers. Preserve pattern syntax in parameters because it is part of the written declaration, but never inspect the body.

Canonical examples:

```rust
pub async fn load<T>(id: Id, source: &T) -> Result<User, T::Error>
where T: Repository<User>;

impl User {
    pub const fn id(&self) -> Id;
}

impl Display for User {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result;
}
```

## 13. Diff engine

Use `similar` over canonical projected text. Configure its patience algorithm and line-based comparison. Use three context lines initially.

Requirements:

- Diff each repository path independently.
- Emit files in raw path byte order.
- Emit no header for a file whose projections are equal.
- Treat an absent projection as empty content.
- Preserve the standard `+`, `-`, and space prefixes.
- Produce plain, deterministic output in core; apply ANSI styling only in the CLI output layer.
- Color deletion red, insertion green, hunk headers cyan, and file headers bold when color is active.
- `--color=auto` uses terminal detection; redirected output contains no ANSI sequences.

Do not implement declaration rename or move detection. Do not compare syntax trees directly. Canonical projection is the semantic step; the diff remains textual.

## 14. CLI contract

The binary name is `ownai`.

Commands:

```text
ownai show --mode <types|signatures> [REVISION]
ownai diff --mode <types|signatures> <BASE> <TARGET>
```

Rules:

- `show` defaults `REVISION` to `HEAD`.
- `--mode` is required; do not introduce a default before product validation.
- Both diff revisions are required.
- Support `--color <auto|always|never>` with `auto` as the default.
- `--help` must describe that implementation-only changes are invisible.
- Successful commands exit `0`, including a diff with projected changes.
- Usage errors exit `2` through `clap`.
- Repository, revision, object, UTF-8, or parse failures exit `1`.
- Write projections and diffs to stdout.
- Write diagnostics to stderr.
- Do not add progress output in the MVP.

The CLI constructs the two projectors, opens `ownai-git`, and implements the Git-aware pipeline in `command.rs`, composing `ownai-git`'s snapshot reads with `ownai-core`'s pure `show` and `diff` rendering. `ownai-core` stays Git-free (section 3). Business rules do not belong in `main.rs`.

## 15. Diagnostics and failure behavior

Diagnostics must include, when applicable:

- Repository location.
- User-provided revision string.
- Repository-relative source path.
- Language.
- Source range.
- Underlying error chain.

Fatal cases include:

- No repository found.
- Revision not found, ambiguous, a range, or not peelable to a commit.
- Git object missing or corrupt.
- Supported source blob is not UTF-8.
- Tree-sitter cannot parse a supported source file without error nodes.
- Adapter finds an AST shape that violates its invariants.

Unsupported extensions, symlinks, submodules, and macro-generated declarations are exclusions, not fatal errors.

Never silently fall back to raw source. Never emit a partial diff after a fatal projection error.

## 16. Testing

### 16.1 Grammar smoke tests

For each grammar:

- Construct a parser.
- Assign the grammar.
- Parse the smallest valid file.
- Assert that the root contains no errors.

### 16.2 Fixture organization

Each fixture case should contain source and expected projections:

```text
fixtures/elm/type-alias/
  input.elm
  types.txt
  signatures.txt

fixtures/rust/trait-impl/
  input.rs
  types.txt
  signatures.txt
```

Required Elm cases:

- Normal and port modules.
- Parameterized custom types.
- Constructors with zero and multiple arguments.
- Aliases of primitives, functions, tuples, and records.
- Multiline type annotations.
- Annotated and unannotated functions.
- Top-level values.
- Ports and infix declarations.
- Nested `let` functions that must be excluded.
- Comments and formatting variations.

Required Rust cases:

- All struct forms and field visibilities.
- Enums with all variant forms.
- Unions and aliases.
- Traits with associated types, methods, defaults, supertraits, and constraints.
- Inherent and trait implementations.
- Free, async, const, unsafe, and extern functions.
- Methods with all receiver forms.
- Constants, statics, and foreign blocks.
- Generics, const generics, lifetimes, and `where` clauses.
- Nested modules.
- Attributes and documentation removal.
- Closures and local items that must be excluded.
- Macro definitions and invocations that must be excluded.
- Comments and formatting variations.

Fixture and test file location:

- Unit tests live beside the code they cover.
- Integration tests live under `crates/<crate>/tests/` and may share helpers
  through a `tests/support/` module that each test target includes with
  `mod support;`.
- Repository-level language fixtures stay under the top-level `fixtures/`
  directory.

### 16.3 Invariance tests

For both languages, prove:

- Changing only a function body leaves both projections unchanged.
- Changing comments leaves both projections unchanged.
- Changing whitespace leaves both projections unchanged.
- Changing a type changes Types and Signatures projections.
- Changing a function signature changes only Signatures output.
- Adding a private function changes Signatures output.
- Reordering declarations changes projected order.

### 16.4 Git tests

Create real temporary repositories and commits. Test:

- SHA-1 repositories and SHA-256 repositories when supported by the test environment.
- `HEAD`, branch, tag, full ID, abbreviated ID, `^`, and `~` resolution.
- Annotated tag peeling.
- Packed objects.
- Added, deleted, modified, and unchanged supported files.
- Mixed Elm and Rust commits.
- Unsupported files ignored.
- Symlink and submodule entries ignored.
- Bare repository operation.
- Invalid and ambiguous revisions.

Create the temporary repositories with the `git` executable. This is the only
place the test suite may invoke Git: test setup may create commits, tags,
branches, worktrees, and bare clones, while library and CLI code must never
invoke the Git executable. Isolate every invocation from host configuration and
the network by pointing `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` at
`/dev/null`, setting `GIT_CONFIG_NOSYSTEM`, disabling commit and tag signing,
and supplying fixed author, committer, and date values. After fixtures exist,
exercise only `ownai-git`. SHA-256 cases must perform a runtime capability check
and skip cleanly when the environment's Git cannot create a SHA-256 repository.

### 16.5 End-to-end CLI tests

Assert stdout, stderr, and exit status for every command form. Snapshot plain output with color disabled. Add a focused assertion that redirected or `--color=never` output has no escape bytes.

## 17. Performance constraints

Correctness and stable output take priority over concurrency in the MVP.

Initial performance rules:

- Skip files with identical blob IDs before reading them during diff.
- Read each needed blob at most once per command.
- Project each `(blob ID, language, mode)` at most once per command.
- Use the bounded `gix` object cache.
- Do not build compiler projects or invoke external processes.
- Do not enable parallel projection until deterministic tests and profiling exist.

Add benchmarks for large synthetic trees and representative real Elm/Rust repositories before adding threads, persistent caches, or broader `gix` features.

## 18. Security and robustness

- Treat repositories and source files as untrusted input.
- Do not execute repository configuration, hooks, filters, attributes, macros, build scripts, compilers, or formatters.
- Do not evaluate shell commands.
- Do not follow repository symlinks.
- Bound any configurable caches.
- Avoid recursion that is proportional to untrusted expression depth where Tree-sitter traversal can be iterative.
- Report allocation or parser failures rather than panicking.
- Reserve `panic!`, `unwrap`, and `expect` for tests or statically guaranteed initialization only.

Fuzzing Tree-sitter itself is out of scope, but adapter traversal and canonical token rendering should be structured so they can receive arbitrary byte input in later fuzz targets.

### 18.1 Required development checks

The repository must provide one documented command, task, or script that runs the following checks without changing tracked files:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo tree -e features -p ownai-git
```

Review the final command whenever dependencies change. It must show no unintended `gix` feature beyond the approved list and unavoidable transitive implications of those features.

## 19. Implementation sequence

Implement in this order. Do not begin a later phase while the current phase's acceptance gate is failing.

### Phase 1: Workspace and core contracts

- Create the workspace and crates.
- Add dependency boundaries.
- Define modes, languages, repository paths, spans, projected items/files, diagnostics, and projector trait.
- Add formatting, linting, and unit-test commands.

Acceptance gate: the workspace builds and `ownai-core` has no Git, parser, or CLI dependencies.

### Phase 2: Git snapshots

- Implement repository discovery with `gix`.
- Implement single-revision resolution and peeling.
- Implement supported-blob tree traversal and reads.
- Add temporary-repository integration tests.

Acceptance gate: tests can enumerate and read Elm/Rust files from two commits without invoking the Git executable.

### Phase 3: Elm projector

- Add grammar smoke test.
- Implement module, type, alias, annotation, value, port, and infix extraction.
- Implement canonical Elm rendering.
- Add every required Elm fixture and invariance test.

Acceptance gate: all Elm snapshots pass, including formatting/body invariance and missing-annotation behavior.

### Phase 4: Rust projector

- Add grammar smoke test.
- Implement Rust type declarations, traits, implementation containers, functions, methods, associated items, constants, statics, foreign blocks, attributes, and nesting.
- Implement canonical Rust rendering.
- Add every required Rust fixture and invariance test.

Acceptance gate: all Rust snapshots pass, including formatting/body invariance and macro exclusion.

### Phase 5: Project rendering and diff

- Implement deterministic project/file ordering.
- Implement show framing.
- Implement patience unified diff, additions, deletions, and empty diff behavior.
- Add mixed-language projection and diff snapshots.

Acceptance gate: body-only commits produce empty focused diffs; type and signature commits produce the expected mode-specific diffs.

### Phase 6: CLI

- Implement arguments, commands, color behavior, diagnostics, and exit codes.
- Add end-to-end CLI tests.
- Ensure help text states focused-diff limitations.

Acceptance gate: all documented command forms work in normal, bare, Elm-only, Rust-only, and mixed repositories.

### Phase 7: Hardening

- Test packed objects and SHA-256 repositories.
- Run formatter, lints, unit tests, integration tests, and release build.
- Measure representative repository performance.
- Audit dependency features and ensure excluded `gix` subsystems remain disabled.

Acceptance gate: a release build passes all tests and the dependency feature tree matches this document.

## 20. Completion definition

The MVP implementation is complete when:

- Both documented commands operate entirely through `gix` and never invoke Git.
- Elm and Rust projections satisfy every product rule in both modes.
- Formatting-, comment-, and body-only edits produce no focused diff.
- Type and signature edits appear in the correct modes.
- Mixed-language, added-file, and deleted-file comparisons work.
- Invalid revisions and unprojectable supported files fail clearly without partial output.
- Output is deterministic across repeated runs and independent of terminal width.
- All required tests and snapshots pass.
- Only the approved `gix` features are enabled.

## 21. Upstream references

Use primary upstream documentation when an API detail in this design needs confirmation:

- [`gix` crate documentation](https://docs.rs/gix/latest/gix/)
- [`gix` feature flags](https://docs.rs/crate/gix/latest/features)
- [Tree-sitter Rust binding](https://docs.rs/tree-sitter/latest/tree_sitter/)
- [Tree-sitter Elm grammar](https://github.com/elm-tooling/tree-sitter-elm)
- [Tree-sitter Rust grammar](https://github.com/tree-sitter/tree-sitter-rust)
- [`similar` diff crate](https://docs.rs/similar/latest/similar/)
- [`clap` derive reference](https://docs.rs/clap/latest/clap/_derive/)

Pin resolved versions in `Cargo.lock`. When upgrading `gix` or a grammar, read its changelog, inspect feature resolution or `NODE_TYPES`, and run the complete fixture and integration suite before accepting new snapshots.
