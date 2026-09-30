# OwnAI Technical Design

## 1. Purpose and authority

This document records the implemented OwnAI architecture and the contracts
that future changes must preserve. Section 19 retains the original delivery
sequence as historical context.

[PRODUCT.md](./PRODUCT.md) is authoritative for product behavior and scope. If this document conflicts with the product document, follow the product document and update this document in the same change.

The current implementation supports:

- Elm, Haskell, Python, and Rust source files.
- Types and Signatures projection modes.
- Showing a projection for a Git commit.
- Diffing projections from two Git commits in text or JSON; JSON also compares
  the index, tracked worktree, and empty tree.
- Narrowing a projection to selected repository paths on both commands.

OwnAI does not perform type inference, expand macros, inspect function bodies,
or implement Public and Full projection modes. The editor projection surface
(section 14.2) may read the worktree or standard input. JSON snapshot diffs
(section 14.3) may read the index and tracked worktree. All paths are read-only.

## 2. Technical decisions

| Concern | Decision |
|---|---|
| Implementation language | Rust, stable toolchain, Rust 2024 edition |
| Project structure | Cargo workspace with separate core, language, Git, and CLI crates |
| Git access | `gix`, read-only, behind an OwnAI-owned interface |
| Parsing | Tree-sitter with the Elm, Haskell, Python, and Rust grammars |
| Projection | Language-specific extraction into a small shared projection model |
| Rendering | Deterministic, language-specific canonical rendering |
| Diffing | Line-oriented patience diff over canonical projected text |
| CLI parsing | `clap` derive API |
| Errors | Typed library errors with `thiserror`; user-facing reports with `miette` |
| Testing | Fixture and snapshot tests, plus end-to-end CLI tests over temporary Git repositories |

The engine, CLI, and optional terminal frontend are compiled into one
executable. The Neovim plugin invokes that executable; there is no daemon, IPC
protocol, plugin host, LSP client, or MCP server.

## 3. Workspace layout

The workspace is organized as follows:

```text
Cargo.toml
Cargo.lock
crates/
  base/
    src/
      lib.rs
      diagnostic.rs
      diff.rs
      doc.rs
      keys.rs
      language.rs
      model.rs
      outline.rs
      path.rs
      render.rs
      selection.rs
  git/
    src/
      lib.rs
      repository.rs
      revision.rs
      tree.rs
  lang-elm/
    src/
      lib.rs
      extract.rs
      render.rs
      syntax.rs
  lang-haskell/
    src/
      lib.rs
      extract.rs
      render.rs
      syntax.rs
  lang-python/
    src/
      lib.rs
      extract.rs
      render.rs
      syntax.rs
  lang-rust/
    src/
      lib.rs
      extract.rs
      render.rs
      syntax.rs
  engine/
    src/
      lib.rs
      engine.rs
      config.rs
      error.rs
  tui/
    src/
      lib.rs
      app.rs
      highlight.rs
      theme.rs
      fuzzy.rs
      icons.rs
      view/
        mod.rs
        chrome.rs
        tree.rs
        show.rs
        diff.rs
        overlay.rs
        geom.rs
        text.rs
  cli/
    src/
      main.rs
      args.rs
      command.rs
      output.rs
      pathspec.rs
fixtures/
  elm/
  rust/
```

Unit tests live beside the code they cover, and each crate may add a `tests/`
directory for integration tests; expected projection output lives under
`fixtures/` (section 16.2).

Dependency direction is one-way:

```text
cli
  ├── base
  ├── git
  ├── engine
  └── tui (optional, default-on)

tui              ──→ base
                 └──→ engine
engine           ──→ base, git
                 └──→ lang-elm, lang-haskell,
                      lang-python, lang-rust
lang-elm     ──→ base
lang-haskell ──→ base
lang-python  ──→ base
lang-rust    ──→ base
git              ──→ base
```

`base` must not depend on `gix`, Tree-sitter, either grammar, or `clap`. Language-specific Tree-sitter node names must never appear in `base`. `gix` types must never leave `git`.

`engine` is the shared Git-aware application layer both frontends use. It
must not depend on `clap`, `miette`, `ratatui`, or `crossterm`, and it never
renders a document or reads command-line arguments.

`tui` is the terminal frontend (section 21). It must not depend on `clap`
or `miette`, discover repositories, parse arguments, or read `.ownai.toml`; it
receives an `Engine` and a fully-built `Selection` and talks to the terminal
through `ratatui`/`crossterm` only.

## 4. Dependencies

Declare shared dependency versions under `[workspace.dependencies]`. Initially use compatible releases in these major/minor families and commit `Cargo.lock`:

```toml
[workspace.dependencies]
tree-sitter = "0.27"
tree-sitter-elm = "5.9"
tree-sitter-haskell = "0.23"
tree-sitter-python = "0.25"
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
serde = { version = "1", features = ["derive"] }
toml = "1"

# Terminal frontend only. `ratatui` is built without its default feature bundle
# so the enabled feature set stays auditable; the crossterm backend is the only
# backend. `syntect` uses the pure-Rust `fancy-regex` backend, and `two-face`
# supplies bat's syntax grammars; in-crate Tokyo Night night/day themes color
# tokens. `terminal-colorsaurus` asks the terminal for its background color
# (OSC 11) to pick the light or dark flavor at startup.
ratatui = { version = "0.30", default-features = false, features = ["crossterm"] }
crossterm = "0.29"
terminal-colorsaurus = "1"
unicode-width = "0.2"
syntect = { version = "5.3", default-features = false, features = ["default-fancy"] }
two-face = { version = "0.5", default-features = false, features = ["syntect-fancy"] }
```

`serde` and `toml` parse `.ownai.toml` for the shared engine (section 14.1);
they are not a serialization dependency of `gix`, whose `serde` feature stays
disabled (section 4.1).

The terminal dependencies are used only by `tui`, behind the default-on
`tui` feature of `cli`. With `--no-default-features`, none of `ratatui`,
`crossterm`, `terminal-colorsaurus`, `unicode-width`, `syntect`, or `two-face`
may appear in `cli`'s dependency tree; this is enforced by
`just check-workspace-nodefault` (section 18.1). `syntect` uses the
`fancy-regex` backend so the build needs no Oniguruma C toolchain.
`terminal-colorsaurus` shares `libc` and `mio` with crossterm and adds only
`terminal-trx` and `xterm-color`. The premium UI work adds no other dependency:
`ratatui` is still built with only its `crossterm` feature, and the palette and
finder use an in-crate fuzzy matcher.

Use current compatible releases for test-only dependencies:

- `assert_cmd` for CLI tests.
- `predicates` for CLI assertions.
- `tempfile` for temporary repositories.
- `portable-pty` for the terminal frontend's PTY smoke test.

Projection and diff expected output is stored as plain fixture text and compared
directly (section 16.2), which is easier to review than opaque snapshots, so no
snapshot crate is required.

Do not add an async runtime. All MVP work is local and synchronous.

### 4.1 `gix` features

Keep `default-features = false`. Enable only:

- `revision`: revision parsing, peeling, and merge-base support. This brings
  `index` transitively; `git` reads stage-zero index blobs for JSON
  snapshot diffs.
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
- `serde`; no `gix` value is serialized outside `git`.
- `revparse-regex`; the MVP does not promise regex commit-message revision searches.

Review this list whenever `gix` is upgraded because it is pre-1.0 and feature relationships may change.

### 4.2 Tree-sitter features

Use the native Tree-sitter runtime with its default `std` support. Do not enable Tree-sitter's WASM runtime. Load the grammars through `tree_sitter_elm::LANGUAGE`, `tree_sitter_haskell::LANGUAGE`, `tree_sitter_python::LANGUAGE`, and `tree_sitter_rust::LANGUAGE`.

At startup in tests, assert that every grammar can be assigned to a parser. Grammar upgrades must include a review of `NODE_TYPES` and all affected snapshots.

The Haskell grammar's generated parser is large, so it increases build time and binary size noticeably; that cost is accepted for language support. Literate Haskell (`.lhs`) is not supported because the grammar cannot parse it, and CPP is not preprocessed, so a source file using `#if` fails as erroneous syntax under section 9 rather than being expanded.

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
    Haskell,
    Python,
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
    TypeFamily,
    PatternSynonym,
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

`ProjectedFile::canonical_text` must be derived from `items` by a constructor and must not be independently mutable. Keep fields private where doing so enforces this invariant. `ProjectedFile::try_new` is that constructor; it also rejects an unsupported path extension and a `language` that disagrees with `path`, so the stored language can never contradict the extension. `SourceSpan` line and column values are zero-based; CLI diagnostics may convert them to one-based display values.

### 5.1 Repository paths

Git paths are byte strings, not guaranteed UTF-8 operating-system paths. `RepoPath` must own normalized repository-relative bytes and use `/` as the separator. Base it on `bstr::BString` or an equivalent owned byte representation.

Requirements:

- Reject absolute paths and `..` traversal.
- Contain paths with a byte-exact, boundary-aware `RepoPath::is_within` check, so a directory contains itself and its descendants but not a sibling whose name merely shares its prefix (`src` does not contain `src2/x.rs`).
- Sort by raw path bytes for deterministic output.
- Detect `.elm`, `.hs`, `.py`/`.pyi`, and `.rs` using ASCII extension bytes.
- Escape invalid UTF-8 when displaying a path.
- Do not convert a repository path to `PathBuf` merely to inspect a committed tree.

All supported source contents must be valid UTF-8. A supported source blob containing invalid UTF-8 is a fatal projection diagnostic.

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
- Haskell module: declared module name.
- Haskell top-level type, class, family, pattern, or value: module name, item kind, and declared name.
- Haskell class or instance member: container key plus item kind and name. Instance members are keyed by the written method name.
- Python file module: repository path.
- Python class or type declaration: container, item kind, and declared name.
- Python class member: class key, item kind (field, method, or variant), and declared name.

If two declarations produce the same key, append a deterministic source-order ordinal. Never include byte offsets in the primary key because harmless edits before a declaration would destabilize it. The collision suffix is allocated by `base::KeyAllocator`, the single implementation every adapter uses.

### 5.3 Canonical text assembly

`ProjectedFile::canonical_text` is derived from `items` by the constructor and is never independently mutable. Emission is determined solely by an item's `parent_key`:

1. Only items whose `parent_key` is `None` are emitted. Their `canonical_text` fragments are joined by exactly one blank line, and the result ends with exactly one trailing newline. A file with no top-level items has empty canonical text.
2. An item whose `parent_key` is `Some(_)` is an index-only entry. Its text is already contained in the fragment of the ancestor that owns it. It is never emitted as its own top-level block.
3. Every item's `canonical_text` is a self-contained fragment. A top-level fragment includes the canonical rendering of its nested members, indented four spaces per nesting level (section 10). A nested item's text appears exactly once in the file text, inside its ancestor.
4. The `parent_key` relationship is independent of the container naming used in stable keys (section 5.2). Naming a container in a stable key does not suppress emission; only `parent_key == Some(_)` does. An adapter marks each declaration it wants emitted as top-level.
5. Every nested item must have a top-level ancestor, and an adapter must not produce an item whose `parent_key` refers to a non-existent item. `ProjectedFile::try_new`, the only way to construct a file, rejects a dangling, self-referential, or cyclic `parent_key` and a duplicate `stable_key`, so canonical-text assembly can never silently drop an item.

Nested items remain in `items` so that stable keys and later per-declaration comparisons can address them, even though their text is emitted through an ancestor.

### 5.4 Path scoping

`PathScope` is a sorted, deduplicated set of repository path prefixes. An empty `PathScope` matches every path, so "no selection" needs no separate case. `PathScope::matches` accepts a path that equals a prefix or descends from one, using the `RepoPath::is_within` check of section 5.1. Scoping only filters which files are projected; it never changes projection rules.

`PathSelection` is how a caller narrows a projection:

- `All` matches everything.
- `Literals(Vec<RepoPath>)` narrows to explicit paths.
- `Areas(Vec<String>)` names repository-defined areas.

The variants make named areas and literal paths mutually exclusive by construction.

`Area` is a named list of repository paths. `Area::new` rejects an empty name or an empty path list and sorts and deduplicates the paths, so every constructed `Area` is non-empty; `AreaSet` is a name-sorted lookup that rejects duplicate names with `AreaError::DuplicateName`. `SelectionGroup` is either a literal `Path(RepoPath)` or an `Area(Area)`, so a group can never be empty and its label cannot disagree with its paths. `Selection::resolve` is the single entry point from a `PathSelection` plus the repository's `AreaSet`: it validates named areas and records one group per literal path or named area in one step, so callers never resolve a scope and then rebuild its groups by hand. It reports an undefined name as `SelectionError::UnknownArea`; an empty area is impossible by construction rather than a runtime error, and `Selection::new` is infallible for the same reason. The resulting `Selection` pairs the resolved `PathScope` with those groups, so an unsatisfied selection can name what the user asked for and no group can disagree with the scope. Areas are data only and core stays file-format-free: the engine loads them from `.ownai.toml` (section 14.1) and passes the resulting `AreaSet` to `Selection::resolve`, so `All`, literal paths, and named areas are all reachable at the frontend boundary.

## 6. Language adapter interface

Expose this conceptual interface from `base`:

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

The pipeline is implemented once in `engine` and shared by the command
line and the terminal frontend (section 21). The two frontends differ only in
how they build the initial selection and present the result.

### 7.1 Show

```text
repository path + revision + path selection
  → discover repository
  → resolve revision to one commit
  → traverse commit tree
  → select .elm, .hs, .py, .pyi, and .rs blobs
  → reject selected paths absent from the revision (before any blob read)
  → keep only paths matching the path scope
  → read each remaining blob
  → project with its language adapter
  → sort files by raw repository path
  → render file sections
  → write output
```

### 7.2 Diff

```text
base revision + target revision + path selection
  → resolve both to commits
  → traverse both trees
  → union supported paths
  → reject selected paths absent from both revisions (before any blob read)
  → for each path in byte order:
      path outside the scope    → skip
      unchanged blob ID         → skip
      old blob only             → project old; compare with empty
      new blob only             → compare empty with projected new
      both blobs                → project both; compare projections
  → emit only projected files with differences
```

A source blob change that produces the same projection produces no output. This is the defining invariant of focused diffing.

For `diff --format json`, the engine resolves each side as a commit, `:index`,
`:worktree`, or `:empty`. The canonical Git empty-tree object ID is an alias
for the empty side. It projects supported paths in raw path-byte order and
omits equal canonical projections. Index entries come from stage-zero blobs;
worktree entries come from tracked regular files on disk without following
symlinks. Mutable snapshot names are labels rather than content hashes.

## 8. Git layer using `gix`

`git` is read-only. It owns repository discovery, revision resolution,
commit peeling, tree traversal, stage-zero index enumeration, and blob reads.

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
    fn path_exists(&self, revision: &Revision, path: &RepoPath)
        -> Result<bool, GitError>;
    fn read_blob(&self, id: &ObjectId) -> Result<Vec<u8>, GitError>;
}
```

`path_exists` reports whether a path names a tree or blob in a revision. A selected path is valid for a diff when either the base or the target names it, so a file deleted by the target stays in scope.

The concrete implementation wraps `gix::Repository`, but callers must not see that type.

### 8.1 Repository discovery

- Begin discovery at the CLI's current directory.
- Support normal repositories, bare repositories, and linked worktrees when `gix` can discover them.
- Open the repository read-only.
- Respect `gix` repository trust handling; do not override trust to force-load unsafe configuration.
- Return a clear diagnostic when no repository is found.

### 8.2 Revision resolution

Use `gix` revision parsing and require the result to identify one object. Peel annotated tags and other commit-ish objects to a commit.

Supported commit forms:

- Full and unambiguous abbreviated object IDs.
- `HEAD`.
- Local branch names.
- Tag names.
- Parent and ancestor suffixes such as `HEAD^` and `HEAD~3` when supported by `gix` revision parsing.

Reject ranges such as `A..B` and `A...B` when passed as one argument. The `diff` command takes two independent revision arguments. Reject a tree or blob that cannot peel to a commit.

### 8.3 Tree traversal

- Recursively visit each commit tree.
- Retain regular and executable blob entries for supported Elm, Haskell,
  Python, and Rust paths (including Python `.pyi` stubs).
- Ignore directories after descending into them.
- Ignore symlinks, Git links/submodules, and unsupported file types.
- Do not perform rename detection.
- Do not read `.gitignore`; committed trees are authoritative.
- Return entries sorted by raw repository path bytes.

Use object IDs to avoid reading or projecting unchanged blobs during diff.

### 8.4 Object caching

Configure a bounded `gix` object cache suitable for repeated tree and blob access. Keep cache sizing in `git` and use a conservative constant initially. Do not expose tuning flags in the MVP. Add benchmarks before changing cache strategy or enabling broader `gix` performance features.

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

Line breaks are fixed by declaration shape and by a single compile-time width budget. Terminal width must never change output. A declaration may wrap only at `base::LINE_WIDTH` (80 display columns), so a focused diff stays readable side by side while remaining deterministic:

- One simple declaration or signature per line, unless the whole line would exceed the budget.
- When a declaration line exceeds the budget, its primary bracketed list (a function's parameters, or a class's superclass list, else its type parameters) breaks one item per indented line, with the closing bracket returned to the declaration's indent. The fit is measured for the whole line, including the terminator or opening brace, so a long return type still breaks the parameter list. Any earlier list breaks only when it does not fit on its own.
- A struct-like enum variant whose field list does not fit breaks one field per indented line.
- An Elm or Haskell type signature whose arrow chain (`->`, and a leading `=>`) does not fit breaks before each arrow, one per indented line.
- A bracketed construct nested inside a type breaks one item per indented line when its own flat form does not fit, and re-decides at its own column after an enclosing list breaks: a Python subscript or `|` union, a Rust generic argument list, tuple, reference, or `Fn(...)`, a Haskell `parens`/`tuple`/`list`/`apply`, or an Elm record or tuple.
- A multi-argument Rust attribute (`#[command(...)]`, `#[arg(...)]`) or Python decorator (`@app.get(...)`) whose argument list does not fit breaks one argument per indented line.
- A Haskell `type` synonym whose right-hand side does not fit breaks after `=` and then before each type operator, one operator-led item per indented line.
- Atomic tokens are never split: a line dominated by one long string literal, identifier, or macro attribute body may still exceed the budget, exactly as a conventional formatter leaves it.
- A broken Python or Rust bracketed list emits a trailing comma; an inline list does not.
- Haskell and Elm never emit a trailing comma: broken lists and records use a leading-comma block (`( a\n, b\n)`), matching their existing record style.
- One union constructor, enum variant, struct field, or record field per indented line when a declaration has a body.
- One trait or implementation member per indented block.
- Blank line between top-level projected declarations (see section 5.3 for how top-level items assemble into file text).
- Four spaces per nesting level.
- Exactly one trailing newline per projected file.

Build a small internal document representation such as `Text`, `Line`, `Indent`, and `Concat`, plus a width-aware `Group` with soft line breaks for the two shapes above, or equivalent direct rendering helpers. This grammar-agnostic layout document lives in `base` as `base::Doc` (module `doc.rs`), shared by every adapter; `base` stays free of Tree-sitter, and each adapter lowers its own parsed tree into `Doc`. Do not add a complete source formatter.

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
expression stays inline as `{ name : Type, ... }` only while it fits; otherwise
it breaks one field per indented line with leading commas and no trailing
comma. Records in type annotations and constructor argument positions wrap the
same way.

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

Alongside `unified_hunks`, core exposes a structured side-by-side alignment:
`aligned_rows(old, new)` returns ordered rows carrying optional old and new line
numbers, old and new text, and an `Equal`/`Add`/`Delete`/`Change`
classification. It shares `unified_hunks`'s patience configuration and line
tokenization, so the two always identify the same changed lines. The terminal
frontend renders side by side from `aligned_rows` and must never derive a
side-by-side view by parsing unified-diff text.

## 14. CLI contract

The binary name is `ownai`.

Commands:

```text
ownai show --format <text|json> --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai diff --format <text|json> --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

Rules:

- `show` defaults `REVISION` to `HEAD`.
- `show` accepts `--format <text|json>` (default `text`) and the
  `--stdin`/`--worktree` input forms; section 14.2 defines the JSON document.
- `--mode` is required; do not introduce a default before product validation.
- Both diff revisions are required. Text diff requires commits; JSON diff
  additionally accepts `:index`, `:worktree`, and `:empty`.
- `--path`/`-p` and `--area`/`-a` are mutually exclusive; passing both is a usage error that exits `2` through `clap`. Each is individually repeatable, and a repeated option forms a union of its selections.
- `--path`/`-p` narrows the projection to the named files or directories; a directory includes every file beneath it. Matching is byte-exact and boundary-aware.
- Paths resolve relative to the current directory. An absolute path must be inside the repository, `..` may climb but may not leave it, and in a bare repository relative paths resolve against the repository root. Resolution is lexical and never consults the filesystem.
- `--area`/`-a` selects named areas defined at the repository root (section 14.1). Area paths are repository-root-relative, so `--area` ignores the current directory.
- A selected path that names nothing in the projected revision exits `1` with empty stdout; for `diff`, a path named by either the base or the target is valid.
- A selected path that exists but contains no supported files exits `0` with empty output.
- An area is satisfied when any one of its paths names something in the projected revision, or in either side of a diff; otherwise it exits `1` with empty stdout and a diagnostic naming the area.
- A missing, unreadable, oversized, or malformed config, or an `--area` name the config does not define, exits `1` with empty stdout. Config errors never affect `--path` or unscoped runs.
- Support `--color <auto|always|never>` with `auto` as the default.
- `--help` must describe that implementation-only changes are invisible.
- Successful commands exit `0`, including a diff with projected changes.
- Usage errors exit `2` through `clap`.
- Repository, revision, object, UTF-8, or parse failures exit `1`.
- Write projections and diffs to stdout.
- Write diagnostics to stderr.
- Do not add progress output in the MVP.

The Git-aware pipeline lives in `engine`, which composes `git`'s
snapshot reads with `base`'s pure rendering and exposes `show` and `diff`
over a `Selection`. The CLI builds that selection from `argv`, invokes the
engine, and renders the result or a diagnostic; `main.rs` stays a thin entry
point. `base` stays Git-free (section 3). Business rules do not belong in
`main.rs`.

### 14.1 `.ownai.toml` and named areas

A named area is a repository-defined group of paths, selected with `--area`. It lets a project share a recurring selection without repeating `--path` arguments.

The config file is `.ownai.toml` at the repository root: the worktree root for a normal repository or linked worktree, and the bare repository root for a bare repository. It maps area names to lists of repository-root-relative paths:

```toml
[areas]
frontend = ["apps/web", "packages/ui"]
backend  = ["services/api"]
```

Rules:

- The file is read lazily, only when `--area` is present. It is read at most once per command, and it never affects `--path` or unscoped runs.
- The on-disk shape is a single `[areas]` table of `name = [paths]`. Unknown keys are rejected, so a typo cannot silently drop the area it was meant to define.
- Area paths are repository-root-relative. Unlike `--path`, they do not depend on the current directory.
- Parsing lives in `crates/engine/src/config.rs`. `base` stays free of file formats: the engine turns TOML into an `AreaSet` and passes it to core's `Selection::resolve` (section 5.4).
- The config is untrusted input. It is size-bounded to 1 MiB, a symlinked config is rejected rather than followed, and an empty or whitespace-only area name or an empty path list is rejected.
- A path that is absolute, contains `..`, or is otherwise unusable is rejected. `.`, repeated slashes, and a trailing slash are normalized leniently; normalization is lexical and never consults the filesystem.
- `--area` and `--path` are mutually exclusive: the `PathSelection` variants make the two selections disjoint by construction, and `clap` rejects a command that passes both with a usage error (exit `2`).
- An area is satisfied when any one of its paths names something in the projected revision, or in either side of a diff. A group that matches nothing is fatal; a group that exists but contains no supported files succeeds with empty output.
- A missing, unreadable, oversized, or malformed config is fatal (exit `1`, empty stdout). An unknown area name is likewise fatal and its diagnostic lists the known names.

### 14.2 Editor projection surface (`ownai.show.v1`)

`ownai show --format json` emits a stable, versioned JSON document so an editor
can fold a real source buffer by OwnAI's declaration structure. `--format`
defaults to `text`, whose output is byte-for-byte unchanged. The schema is
committed at `docs/schema/ownai.show.v1.json`; golden documents live under
`fixtures/schema/`.

Input forms:

```text
ownai show --format json --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai show --format json --mode <types|signatures> --stdin --path <PATH>
ownai show --format json --mode <types|signatures> --worktree --path <PATH>
```

- `--stdin` reads source bytes from standard input; `--worktree` reads the file
  at `--path` from disk. Both require exactly one `--path`, which supplies the
  language and the repository-relative path used to build stable keys, and both
  are mutually exclusive with `REVISION` and `--area`. A missing or repeated
  `--path` is a usage error (exit `2`), and so is a `--path` that names a
  directory: `.` and the repository root resolve to a directory scope, and an
  existing directory such as `src` is rejected because an editor buffer names a
  single file.
- The path is resolved with the same lexical containment rules as `--path`
  scoping (section 14). `--worktree` additionally refuses a symlinked target and
  checks the fully-resolved path stays inside the repository, so a read can
  never leave it even through a symlinked ancestor directory. An unsupported
  extension, a non-UTF-8 source, a path outside the repository, a symlinked
  worktree file, or a resolved path that escapes exits `1` with empty stdout.
- These inputs are read-only. They never write the repository, worktree, or
  index.
- JSON is written raw; `--color` never introduces ANSI into it.

Document shape:

```jsonc
{
  "schema": "ownai.show.v1",   // a consumer treats any other value as fatal
  "input": "revision",          // "revision" | "stdin" | "worktree"
  "revision": "HEAD",           // string, or null for stdin/worktree
  "mode": "types",              // "types" | "signatures"
  "files": [                    // raw path byte order for revisions
    {
      "path": "src/auth.rs",
      "language": "rust",
      "projection": {
        "text": "<the exact canonical text text mode would emit>",
        "items": [ { "stable_key", "parent_key", "kind", "name", "span", "canonical_text" } ]
      },
      "outline": [
        {
          "stable_key": "impl Session::method::refresh",
          "parent_key": "impl Session",
          "kind": "method",
          "name": "refresh",
          "span": { "start_line": 40, "end_line": 58, "start_byte": 1024, "end_byte": 1580 },
          "signature": "fn refresh(&mut self, token: Token) -> Result<(), Error>;",
          "retained_in_mode": true
        }
      ]
    }
  ]
}
```

Contract rules:

- `outline` is mode-independent and complete. It is derived from the Signatures
  projection (the superset): the requested mode supplies `projection`, and
  `retained_in_mode` is set by `stable_key` membership. Types can drop an entire
  `impl` block, so projected items alone cannot locate folds. A corpus test
  proves the Signatures projection is a superset of Types by `stable_key`.
- The closed-fold text of a declaration depends on `retained_in_mode`. When the
  requested mode **retains** the `stable_key`, the mode-correct text is the
  matching `projection.items[].canonical_text`. When it does not, use
  `signature`, which is the declaration's canonical fragment from the Signatures
  superset and is intentionally the superset form. The two differ for container
  declarations (trait/impl/module): for example, a trait implementation that
  Types mode retains with only its associated types has a `signature` that also
  shows the methods Signatures mode adds. A nested declaration's fragment
  carries its container indentation, and it is single line where the canonical
  form is single line.
- `projection.text` is exactly the canonical text text mode emits for that file,
  so the document and the text view cannot drift.
- Line numbers are one-based on the wire (the JSON layer adds one to the
  zero-based adapter span), for editor friendliness; byte offsets are zero-based
  into the decoded UTF-8 source. A consumer converts byte offsets if needed and
  does not convert line numbers.
- `span` starts at the declaration node, so preceding attributes, decorators,
  `{-# ... #-}` pragmas, and doc comments are excluded even though `signature`
  may include them. An editor extends a fold start upward over those lines. A
  `decorator_start_line` field can be added additively within `ownai.show.v1`
  later.
- `stable_key` is unique within its file but not necessarily across the
  repository: Rust `impl` keys are not path-namespaced, so a consumer keys
  global state (expanded folds, cursors) by `(path, stable_key)`.
- `kind` is an exhaustive mapping of `ItemKind` with no wildcard arm, so a new
  kind is a compile error rather than a silent fallback.
- Unknown fields must be tolerated by consumers, and fields may be added
  additively within `ownai.show.v1`; the schema permits additional properties
  while keeping the required fields and value constraints strict. A `schema`
  value other than `ownai.show.v1` is a fatal, explicit version mismatch.

Crate placement:

- `base` stays Git-free and serialization-free. It owns the outline model
  (`OutlineItem`, `FileOutline`) and the pure `assemble_outline` derivation from
  the existing `ProjectedItem` model, alongside `FileDiff` and `FileOutlineDiff`.
- `engine` owns the Git-aware outline operations: `Engine::show_outlines`
  for a committed revision and the free `project_source(RepoPath, bytes, mode)`
  for editor-supplied bytes, which needs no repository. `CommitDiff` and
  `SnapshotDiff` stay here because they carry Git snapshot identities.
- `cli` owns the JSON types (`src/json.rs`) and serialization. The schema
  and golden fixtures are referenced by the CLI test suite, never by
  `base`.

### 14.3 Focused snapshot diff document (`ownai.diff.v1`)

`ownai diff --format json` emits the schema in
[`docs/schema/ownai.diff.v1.json`](./schema/ownai.diff.v1.json). The root
contains `schema`, `mode`, `base`, `target`, and `files`. Each snapshot has
a `kind` (`commit`, `index`, `worktree`, or `empty`), the requested
`revision`, and an `id`. A commit ID is resolved; `:index` and
`:worktree` IDs are mutable labels, so callers must refresh after staging or
disk writes.

Each changed file has an escaped repository `path`, `language`, `status`
(`added`, `deleted`, or `modified`), `base` and `target` sides, and
`equal: false`. An absent side is `null`; a present side contains its
`snapshot_id`, canonical `projection`, and declaration `outline`. Files
whose projected text is equal are omitted. The stage-zero index supplies
`:index`; `:worktree` reads tracked regular files from disk and does not
include unsaved editor buffers or untracked files. Text diff retains its
commit-only behavior.

The in-repository Neovim plugin uses this document for Diffview. Its adapter
is guarded by hashes of a specific Diffview revision because it touches
private Diffview internals. See
[`editors/nvim/README.md`](../editors/nvim/README.md) for the pin and
supported workflows.

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
- A selected path that names nothing in the projected revision, or in either side of a diff.
- `--area` selection with a missing, unreadable, oversized, or malformed `.ownai.toml`.
- `--area` naming an area the config does not define; the diagnostic lists the known names.
- An empty area (no paths) or an area path that is absolute or contains `..`.
- Git object missing or corrupt.
- Supported source blob is not UTF-8.
- Tree-sitter cannot parse a supported source file without error nodes.
- Adapter finds an AST shape that violates its invariants.

Unsupported extensions, symlinks, submodules, and macro-generated declarations are exclusions, not fatal errors. A selected path that exists but contains no supported files is likewise an exclusion: it is valid and yields empty output rather than a diagnostic.

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

Required Haskell cases:

- Module headers with and without export lists, and files with no header.
- `data` and `newtype` declarations, including record, prefix, and infix constructors.
- GADTs and constructors with contexts.
- Type synonyms, kind signatures, and type-role annotations.
- Type and data families, abstract and closed, with instances.
- Classes with superclass constraints, method signatures, and default methods.
- Instances, including method heads and associated family instances.
- `deriving` clauses with strategies and `via`.
- Top-level signatures, annotated and unannotated bindings, and multi-name signatures.
- Foreign imports and pattern-synonym signatures.
- Preserved pragmas.
- Imports, comments, Haddocks, and Template Haskell splices that must be excluded.
- Comments and formatting variations.

Required Python cases:

- Classes with bases, keywords, and PEP 695 type parameters.
- Dataclass-style annotated fields and enum members.
- Type aliases, `TypeVar`/`NewType` declarations, and `.pyi` stubs.
- Functions and methods with annotations, defaults, `async`, and decorators.
- Module-level annotated and unannotated values.
- Nested functions, lambdas, and local classes that must be excluded.
- Imports and docstrings that must be excluded.
- Comments and formatting variations.

Fixture and test file location:

- Unit tests live beside the code they cover.
- Integration tests live under `crates/<crate>/tests/` and may share helpers
  through a `tests/support/` module that each test target includes with
  `mod support;`.
- Repository-level language fixtures stay under the top-level `fixtures/`
  directory.

### 16.3 Invariance tests

For every language, prove:

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
- Mixed-language commits.
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
exercise only `git`. SHA-256 cases must perform a runtime capability check
and skip cleanly when the environment's Git cannot create a SHA-256 repository.

### 16.5 End-to-end CLI tests

Assert stdout, stderr, and exit status for every command form. Snapshot plain output with color disabled. Add a focused assertion that redirected or `--color=never` output has no escape bytes.

Path scoping has its own end-to-end coverage over temporary repositories: scoping to a directory, to a single file, and to a union of paths; `.` inside a subdirectory; an absolute path inside the repository; rejection of a path that would leave the repository; added and deleted files in a scoped diff; and a runtime capability check that skips the non-UTF-8 committed-path case when the environment cannot create one.

Named areas have end-to-end coverage over temporary repositories: selecting an area and matching the scoped output, resolving an area from a subdirectory to prove paths are repository-root-relative, repeating `--area` for a union, an unknown area exiting `1` with the known names, a missing and a malformed config exiting `1` only when areas are selected, the `--area` + `--path` usage error exiting `2`, and area-level existence where any one existing path satisfies the group.

`config.rs` unit tests cover the schema: a valid file, a missing file, malformed TOML, an unknown top-level key, a duplicate area key, an empty area name, an empty path list, an absolute path, a `..` path, lenient `.`/trailing-slash normalization, a file with no `[areas]` table, an oversized file, and a symlinked file.

### 16.6 Terminal frontend tests

- `update` and `view` are pure: tests drive `update` with `Msg` sequences and
  assert the resulting model and effects, and assert that an engine call leaves
  `update` as a `Cmd` rather than being performed inline.
- `ratatui::TestBackend` covers the file tree, empty states, help, diagnostics,
  responsive layouts, the modal overlays, syntax coloring, themed diff
  backgrounds, intra-line emphasis, hunk bands, and side-by-side wrapping. A
  coverage test renders both flavors and asserts no cell keeps the terminal's
  default background, so the palette is proven to own the whole canvas.
- Palette, finder, and search are covered by pure `update` tests (filtering,
  action dispatch, selection, live matches, stepping) and by rendered
  `TestBackend` tests. The fuzzy matcher has its own unit tests, including a
  scoring order between consecutive and scattered matches.
- `theme.rs` unit tests cover capability resolution (truecolor, 256, 16, and
  `NO_COLOR`), Tokyo Night night/day palette roles, and the `OWNAI_THEME` override:
  `dark`/`light` are explicit and unknown values defer to the terminal query.
- An injected terminal driver covers partial setup and matching cleanup; a PTY
  smoke test runs where the platform supports one. The PTY test waits for the
  startup color query to finish before sending its quit key, since the query
  reads standard input.
- The feature graph is checked by `just check-workspace-nodefault`: the
  no-default-features build must not link `ratatui`, `crossterm`,
  `terminal-colorsaurus`, `syntect`, or `two-face`.
- No test launches an editor, writes repository data, or depends on the
  developer's terminal configuration.

### 16.7 JSON and Neovim integration tests

The CLI tests compare `ownai.show.v1` and `ownai.diff.v1` documents against
golden fixtures and their committed JSON schemas. Snapshot diff tests cover
commit, index, worktree, and empty inputs, including staged and unstaged
changes. `just test-nvim` runs the plugin's headless Lua suite against the
built binary. `nix flake check` includes the same plugin suite in a temporary
Git repository. Diffview integration tests additionally require the pinned
Diffview checkout described in the plugin README and are run separately.

## 17. Performance constraints

Correctness and stable output take priority over concurrency in the MVP.

Initial performance rules:

- Read `.ownai.toml` at most once per command and only when `--area` is present, so unscoped and `--path` runs never touch it.
- Filter entries by path scope before reading blobs, so scoping bounds the number of blobs read and projected.
- Skip files with identical blob IDs before reading them during diff.
- Read each needed blob at most once per command.
- Project each `(blob ID, language, mode)` at most once per command.
- Use the bounded `gix` object cache.
- Do not build compiler projects or invoke external processes.
- Do not enable parallel projection until deterministic tests and profiling exist.

Add benchmarks for large synthetic trees and representative real repositories before adding threads, persistent caches, or broader `gix` features.

The terminal frontend follows the same rules. It holds large, rarely-changed
collections (`Content`, tree rows, visible paths) behind `Arc`, so cloning the
TEA model per message is O(1) rather than proportional to repository size.
Syntax highlighting is computed once per selected file and cached by path, and
the wrapped diff layout is cached and invalidated by content generation, size,
selection, and tree width. It keeps no persistent cache, and engine caches stay
per-operation.

Frontend responsiveness work is deferred until it is measured. If input-to-redraw
latency exceeds roughly 100 ms median or 200 ms p95 on the reference host, add a
cancellable worker and a loading state before considering persistent caches.

### Performance baseline

Measured 2026-09-21 with rustc 1.98.1 (`48a229cea 2026-09-01`) and cargo 1.98.1,
running `target/release/ownai` on an Apple Silicon macOS host. Times are wall
clock for `--mode signatures`, best and median of ten warm runs. These are a
baseline for later comparison, not a target.

| Command | Repository | Supported files | Elapsed (best / median) |
|---|---|---|---|
| `show --mode signatures HEAD` | synthetic, 150 Elm + 150 Rust | 300 | 33 ms / 34 ms |
| `diff --mode signatures <base> <target>` | synthetic, every file changed | 300 | 53 ms / 55 ms |
| `show --mode signatures HEAD` | OwnAI itself | 67 | 70 ms / 76 ms |
| `diff --mode signatures acf05df a0d42f5` | OwnAI itself | 67 changed | 74 ms / 85 ms |

The synthetic repository is packed (`git repack -a -d` plus
`git prune-packed`) and carries a 100-commit front-loaded history. No threads,
persistent caches, or broader `gix` features were added to obtain these numbers.

### Frame rendering benchmark

`tui` carries a criterion benchmark for a single rendered frame. It sits
behind the `bench` feature, which exposes a `#[doc(hidden)]` façade, so
`criterion` never enters a normal build or the `--no-default-features` CLI build:

```text
just bench-tui
# or: cargo bench -p tui --features bench --bench frame
```

It reports per-frame totals and the seams they are made of:

- `frame/warm/*` reuses one terminal, so ratatui's surface diff is small: an
  idle frame or a small scroll.
- `frame/cold/*` builds a fresh terminal per iteration, so every cell is written:
  the frame after a load or a large jump.
- `update/*` measures message handling, where syntax highlighting and the wrapped
  diff layout are recomputed when the selection or size changes.
- `parts/*` measures each seam alone: `highlight`, `layout_diff`, and ratatui's
  full-surface diff scan (`buffer_diff`).

A warm frame is approximately `view` assembly plus `parts/buffer_diff`; a cold
interaction adds the matching `update/*` cost. Criterion writes an HTML report
under `target/criterion/`.

Measured 2026-09-22 with rustc 1.98.1 (`48a229cea 2026-09-01`) and cargo 1.98.1
on an Apple Silicon macOS host at a quiet moment (load average about 2.0). Host
load moves the few-hundred-microsecond frame numbers by tens of percent between
runs; the slower seams are stable and carry the signal. Times are the median of
30 samples after a one-second warm-up, optimized (`bench`) profile.

| Benchmark | Median |
|---|---|
| `frame/warm/show_two_files` | 0.26 ms |
| `frame/warm/show_long_file` | 0.50 ms |
| `frame/warm/diff_single` | 0.51 ms |
| `frame/warm/show_tiny` | 0.005 ms |
| `frame/cold/diff_single` | 0.74 ms |
| `parts/highlight_rust` (23 lines) | 1.13 ms |
| `parts/highlight_rust_long` (4000 lines) | 192 ms |
| `parts/layout_diff` (800-line diff) | 1.63 ms |
| `parts/buffer_diff` (140x40 surface) | 0.085 ms |

Attribution from the same run:

- A warm 140x40 frame is roughly two-thirds `view` assembly and one-third
  ratatui's full-surface diff scan. The scan is proportional to cells, not to how
  much changed, so a nearly static frame still pays it.
- A cold interaction is dominated by syntax highlighting: syntect costs about
  48 microseconds per Rust line and 21 per Elm line here, so the first view of a
  ~2100-line Rust file exceeds the 100 ms median budget on highlighting alone.
  Per-file caching means this is paid once per file, not per frame.
- Wrapped diff layout (`aligned_rows` plus emphasis, wrapping, and run cloning)
  is about 1.6 ms for an 800-line diff and is cached by generation, size,
  selection, and tree width.

The `update/*` numbers in the next section are the pre-coalescing baseline;
that section measures what changed.

This does not measure escape-sequence encoding or writes to a real terminal;
`TestBackend` replaces them with an in-memory surface. A PTY-based end-to-end
input-to-redraw measurement remains open (plan §6.6).

### Input coalescing and deferred selection work

Arrow-key tree navigation (what a touchpad becomes, since mouse capture is off)
used to draw once per event and, more expensively, highlight every file row the
cursor crossed: `sync_selected` ran syntax highlighting and search re-sync
eagerly, so a 300-row scroll paid 300 cold highlights and cloned a growing cache
on every message.

The runtime now folds every already-queued event into one batch and calls a pure
`settle` once, so a batch highlights only its final selection. The highlight
cache is bounded to 32 entries, and highlighting is skipped when the body is not
drawn (narrow terminal, tree focused).

Measured 2026-09-22 on a busy host (load average about 5.7). The scroll
comparison is within a single run, so it is insensitive to host load; the
`per_step` column reproduces the previous behavior in the same binary and
matches the pre-change `per_key` baseline within 2%.

| Scroll burst | Before (per_step) | After (batched) | Speedup |
|---|---|---|---|
| 1 row | 1.91 ms | 1.91 ms | 1.0x |
| 10 rows | 18.5 ms | 1.86 ms | 10x |
| 100 rows | 175 ms | 1.88 ms | 93x |
| 300 rows | 520 ms | 2.02 ms | 257x |

Message handling, before versus after:

| Benchmark | Before | After |
|---|---|---|
| `update/show_next_file` (20-line file) | 901 us | 1.0 us |
| `update/diff_next_file` | 401 us | 1.1 us |
| `update/diff_resize` (800-line diff) | 1.31 ms | 1.0 us |
| `update/load_show_300_files` | 1.11 ms | 155 us |
| `update/load_diff_40_files` | 481 us | 24 us |

The highlight and layout work did not disappear; it moved to `settle`, where it
runs once per batch instead of once per message. Warm frames are unchanged: the
frame path is untouched, and a back-to-back A/B measured 940 us on the baseline
commit and 911 us after for `frame/warm/show_two_files`.

## 18. Security and robustness

- Treat repositories and source files as untrusted input.
- Do not execute repository configuration, hooks, filters, attributes, macros, build scripts, compilers, or formatters.
- Do not evaluate shell commands.
- Do not follow repository symlinks.
- Bound any configurable caches.
- Avoid recursion that is proportional to untrusted expression depth where Tree-sitter traversal can be iterative.
- Report allocation or parser failures rather than panicking.
- Reserve `panic!`, `unwrap`, and `expect` for tests or statically guaranteed initialization only.

The terminal frontend verifies that standard input and output are terminals
before emitting any control sequence, stages setup so a partial failure is
undone in reverse order, and restores the terminal on normal return, error, and
unwinding panic. Teardown is best-effort and never panics from `Drop`. Terminal
events, and everything the frontend reads, are untrusted input.

Fuzzing Tree-sitter itself is out of scope, but adapter traversal and canonical token rendering should be structured so they can receive arbitrary byte input in later fuzz targets.

### 18.1 Required development checks

The repository must provide one documented command, task, or script that runs the following checks without changing tracked files:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p tui --features bench --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo tree -e features -p git
cargo build -p cli --no-default-features
```

Review the `cargo tree` command whenever dependencies change. It must show no unintended `gix` feature beyond the approved list and unavoidable transitive implications of those features. The final build must succeed and link none of `ratatui`, `crossterm`, `terminal-colorsaurus`, `syntect`, or `two-face`; `just check-workspace-nodefault` additionally asserts their absence with `cargo tree`.

## 19. Implementation sequence

The following sequence records the completed MVP delivery. It is historical
context, not a list of unimplemented work.

### Phase 1: Workspace and core contracts

- Create the workspace and crates.
- Add dependency boundaries.
- Define modes, languages, repository paths, spans, projected items/files, diagnostics, and projector trait.
- Add formatting, linting, and unit-test commands.

Acceptance gate: the workspace builds and `base` has no Git, parser, or CLI dependencies.

### Phase 2: Git snapshots

- Implement repository discovery with `gix`.
- Implement single-revision resolution and peeling.
- Implement supported-blob tree traversal and reads.
- Add temporary-repository integration tests.

Acceptance gate: tests can enumerate and read supported files from two commits without invoking the Git executable.

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

Acceptance gate: all documented command forms work in normal, bare, single-language, and mixed repositories.

### Phase 7: Hardening

- Test packed objects and SHA-256 repositories.
- Run formatter, lints, unit tests, integration tests, and release build.
- Measure representative repository performance.
- Audit dependency features and ensure excluded `gix` subsystems remain disabled.

Acceptance gate: a release build passes all tests and the dependency feature tree matches this document.

### Phase 8: Engine extraction and the Show frontend

- Extract the Git-aware pipeline into `engine` without changing CLI output.
- Add `tui` with the TEA core and terminal runtime, behind the default-on `tui` feature.
- Implement `ownai tui show`: file tree, projection pane, mode switch, scrolling, help, and the terminal-safety contract.

Acceptance gate: `ownai tui show` works in normal, bare, single-language, and mixed repositories; CLI output is byte-for-byte unchanged; the no-default-features build links no terminal dependency.

### Phase 9: Diff frontend

- Add `aligned_rows` to core and render a side-by-side projection diff with synchronized scrolling and long-line wrapping.
- Add syntax highlighting and delta-style diff styling (section 21.6).

Acceptance gate: added, deleted, and modified projected files render correctly; body-only changes stay invisible; wrapping never crosses pane bounds.

### Phase 10: Frontend usability

- Add revision, scope/area, and mode controls; a resizable tree; a structured status bar; and hunk headers.

Acceptance gate: the documented keybindings work end to end, and every earlier frontend gate still passes.

### Phase 11: Haskell and Python support

- Add the `tree-sitter-haskell` and `tree-sitter-python` grammars and the
  `lang-haskell` and `lang-python` crates.
- Extend `Language`, `RepoPath` extension detection, and `ItemKind` in core.
- Implement Haskell and Python extraction and canonical rendering (sections 23
  and 24) with syntax, fixture, projector, and invariance tests.
- Register the adapters in the engine's fixed projector list.
- Add Haskell and Python syntax mapping and Nerd Font glyphs to the frontend.
- Add mixed four-language end-to-end coverage.

Acceptance gate: both new languages satisfy every required fixture and
invariance test; a body-only change in either language produces an empty focused
diff; mixed four-language `show` orders by raw path bytes; the full workspace
check passes.

## 20. Original MVP completion gate (historical)

The original MVP gate required:

- Both documented commands operate entirely through `gix` and never invoke Git.
- Elm, Haskell, Python, and Rust projections satisfy every product rule in both modes.
- Formatting-, comment-, and body-only edits produce no focused diff.
- Type and signature edits appear in the correct modes.
- Mixed-language, added-file, and deleted-file comparisons work.
- Invalid revisions and unprojectable supported files fail clearly without partial output.
- Path scoping on both commands behaves as documented, including the fatal absent-path case.
- Named areas load from `.ownai.toml` only for `--area`, scope both commands by repository-root-relative paths, and fail clearly on a missing or malformed config or an unknown area.
- Output is deterministic across repeated runs and independent of terminal width.
- All required tests and snapshots pass.
- Only the approved `gix` features are enabled.
- The terminal frontend opens, browses, and quits safely in normal, bare,
  single-language, and mixed repositories, restoring the terminal on every
  exit path.
- The frontend switches modes, changes scope and revisions, resizes the tree,
  and renders syntax-highlighted side-by-side diffs with hunk headers.
- Building `cli --no-default-features` succeeds and links no terminal
  dependency.

## 21. Terminal frontend

`ownai tui` is an optional, default-on terminal frontend over the same
projections. `tui` is the only crate that touches the terminal.

### 21.1 Command surface

```text
ownai tui show --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai tui diff range --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
ownai tui diff commits --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

- `show` defaults `REVISION` to `HEAD`; both diff revisions are required.
  Bare `tui diff` is invalid. Range compares the endpoints directly. Commits
  requires the base on the target's first-parent chain and lists the steps
  after the base, newest first. Each selected step compares its first parent
  with the commit. Equal endpoints yield an empty list.
- The feature is default-on: `default = ["tui"]`, `tui = ["dep:tui"]`.
  Without it, `tui` is an unknown command (exit `2`) and no terminal dependency
  is linked.
- The command line owns `argv` conversion and builds the initial `Selection`; it
  never passes `clap` types into the frontend.
- A non-terminal invocation exits `1` with empty stdout and one diagnostic on
  stderr.

### 21.2 The Elm Architecture

`tui` follows The Elm Architecture. `app.rs` holds the pure `Model`,
`Msg`, `Cmd`, and `update`; the pure `view` lives in `view/`; `lib.rs` is the
only imperative layer, owning the run loop, terminal lifecycle, and effect
interpretation.

- **Model** — one plain data structure holds the entire UI state. `update`
  replaces it wholesale rather than editing it in place.
- **Msg** — every input is a value: a key, a resize, a tick, or the completion
  of an effect.
- **Action** — a semantic command. Keys translate to `Action`s and the command
  palette dispatches the same values, so a binding and its palette entry cannot
  drift apart.
- **update** — a pure `(Msg, &Model) -> (Model, Vec<Cmd>)`. It performs no I/O.
- **Cmd** — I/O is described as data. `Load` runs `Engine::show` or
  `Engine::diff`; commit history loads the metadata once and projects the
  selected step lazily. `LoadAreas` reads `.ownai.toml` through
  `Engine::load_areas`.
- **view** — a pure `&Model -> widgets`.
- **settle** — a pure `Model -> Model` that runs the selection-dependent work:
  syntax highlighting, search re-sync, and the wrapped diff layout. The runtime
  calls it once per input batch, after folding every queued message, so a burst
  of navigation pays for its final selection only.
- Effects are transactional: a failed reload keeps the previous model and
  displays a self-expiring diagnostic.

The view is split by surface (`view::chrome`, `tree`, `show`, `diff`,
`overlay`, `geom`, `text`) so each widget stays readable. The split is
presentation-only; state and behaviour remain in `app.rs`.

### 21.3 State sharing and derived cache

Large, wholesale-replaced collections (`Content` payloads, tree rows, visible
paths) are `Arc`-backed, so cloning a `Model` is O(1) rather than proportional
to repository size. Syntax highlighting is cached per selected path in a bounded,
insertion-ordered map (32 entries), so memory and the per-message clone stay flat
however far the user scrolls. The wrapped diff layout is cached in `Derived`,
invalidated by content generation, size, selection, and tree width.
Highlighting, search re-sync, and layout are computed in `settle` once per input
batch, not in `update`. Per-frame data stays in plain `Vec`. The frontend keeps
no persistent cache.

### 21.4 Layout and interaction

- The frame is a one-line header, a content region, and a one-line footer. The
  header carries the brand, repository, and mode/revision/scope chips. The
  footer carries the selected path, line or change counts, search status, and
  key hints that change with the focused pane and active overlay.
- A file tree sits beside a content region, separated by a shared single-column
  divider (panes drop the borders they share, so no border is drawn twice).
  `Tab` toggles between them in show and range views. In commits view, a
  scrollable commit picker sits above the file tree in the left column, capped
  at seven visible rows on normal terminals and reduced on short terminals;
  `Tab` cycles Commits, Files, and Diff. For a diff the two sides are one focus unit, and
  vertical scrolling moves both in lockstep; below 80 columns the sides stack
  vertically when Diff is focused, while Commits or Files takes the full width
  when focused. The same geometry determines drawing and mouse hit testing.
- Modal overlays capture keys until dismissed: help, revision entry, the scope
  chooser, the mode picker, the command palette (`Ctrl-P`), the fuzzy file
  finder (`Ctrl-F`), and search (`/`). Every overlay dims the frame behind it.
  The scope chooser loads areas as an effect and keeps `RepoPath` identity for
  named areas; literal path entry is UTF-8.
- The file tree renders box-drawing guides, diff `A`/`M`/`D` badges, and a
  scrollbar. The focused selection uses a themed highlight with an accent bar;
  the selected file retains a quieter highlight when focus moves to the body.
- Diff panes use short `BASE` and `TARGET` titles with clipped revision labels;
  the selected path lives in the footer. Hunk headers fill the pane with a
  subtle band. Empty tree and content panes center a title and short explanation.
- Search matches are highlighted in both the projection pane and the diff
  panes; `n`/`N` step through them.
- The file-tree width is adjustable and clamped.
- `RepoPath` remains the identity; labels are escaped for display only.
- Mouse capture is enabled. A left click selects a file or folds a directory;
  a click in the commit picker selects a commit; the wheel scrolls the pane
  under the pointer and focuses it. Overlays stay
  keyboard-driven. Only normal tracking and SGR coordinates are turned on, so
  motion and drag never reach the event loop. Because capture takes over the
  terminal's own selection, copying uses the terminal's selection override
  (usually `Shift`-drag).

### 21.5 Terminal lifecycle and safety

- Verify that standard input and output are terminals before emitting any
  control sequence.
- Stage setup (raw mode, alternate screen, cursor, mouse capture) and record
  which steps succeeded; undo exactly those, in reverse order, on normal return,
  error, or unwinding panic. Teardown is best-effort and never panics from
  `Drop`.
- A panic hook restores the terminal before the panic message prints.
- Resize events clamp all selections and offsets, including zero-sized layouts.
- The runtime is generic over an injected driver so setup and cleanup can be
  tested without a real terminal; a PTY smoke test covers the real crossterm
  path where the platform supports one.

### 21.6 Syntax highlighting and diff styling

- Syntax grammars come from `two-face`'s bat assets (Elm, Haskell, Python, and
  Rust included). `syntect` uses the pure-Rust `fancy-regex` backend and
  in-crate Tokyo Night night/day token colors aligned with the UI flavor.
- Diff rows use Tokyo Night semantic added and removed backgrounds, with a
  brighter intra-line emphasis on the bytes that differ, `+`/`-` gutter markers,
  full-width hunk bands, and `⋯ n unchanged lines` indicators for collapsed gaps.
- Hunk headers and the collapsing of long unchanged runs are computed in the
  frontend from `aligned_rows`; core's diff model is untouched.
- `NO_COLOR` disables all styling; structure such as line numbers, continuation
  markers, and hunk headers survives.
- Highlighting is presentation only: canonical projection text and CLI output
  are byte-for-byte unchanged.

### 21.7 Design system and theming

- `theme.rs` holds Tokyo Night night/day semantic tokens (surfaces, borders,
  text, accent, status, selection, diff, search, and tree roles) and resolves
  them through a `Capability`: truecolor, the 256-color xterm palette, the 16 ANSI colors, or
  no color. The theme is part of the `Model`, so `view` never reads the
  environment.
- The dark or light flavor is chosen at startup: `OWNAI_THEME=dark` and
  `OWNAI_THEME=light` are explicit, and anything else (including unset) asks the
  terminal for its background color with `terminal-colorsaurus` over `OSC 11`,
  falling back to dark when the terminal does not answer. `COLORTERM` and `TERM`
  select the capability, and `NO_COLOR` wins over both. The query runs once in
  `run`, before crossterm's lazy event reader is first polled, and briefly
  consumes a keystroke typed during startup.
- `view` paints the whole frame with `palette.bg` and every pane block carries
  that background, so the palette owns every cell and a terminal whose own
  background differs from the palette never shows through.
- Text drawn on a filled cell (chips, the diagnostic toast, the input cursor,
  and a selected palette match) uses `Theme::ink`, which picks whichever of
  `text` or `bg` contrasts more with the fill. This replaces the old
  `palette.bg`-as-ink trick, which was legible in the dark flavor but near-white
  on the light flavor's pale fills.
- The palette and finder use a small dependency-free fuzzy matcher
  (`fuzzy.rs`); no new crate is added for navigation.
- `icons.rs` holds an opt-in Nerd Font glyph set. Nerd Fonts cannot be detected
  from inside a program, so icons default off and are enabled with
  `--icons=nerd` or `OWNAI_ICONS=nerd`. Every glyph is a single display column,
  which keeps the tree's display-width alignment intact; the glyphs come from
  the Font Awesome and Devicons/Seti ranges that exist in Nerd Fonts v2 and are
  aliased in v3.

### 21.8 Loading and feedback

- The runtime reads one event, then folds every already-queued event into a
  batch (bounded by a count and a short time budget), applies them, runs
  `settle` once, and draws once. The blocking first read is unchanged, so a
  single keypress gains no latency, while a burst of arrow-key scrolling becomes
  one frame instead of one per row.
- The runtime polls for input and emits `Msg::Tick` when idle. A tick advances
  the spinner and expires a diagnostic; an idle tick with nothing to animate
  neither updates nor redraws.
- Emitting a `Load` marks the model busy, and the runtime draws the busy frame
  before the blocking effect runs, so the spinner is visible during a
  projection.

## 22. Upstream references

Use primary upstream documentation when an API detail in this design needs confirmation:

- [`gix` crate documentation](https://docs.rs/gix/latest/gix/)
- [`gix` feature flags](https://docs.rs/crate/gix/latest/features)
- [Tree-sitter Rust binding](https://docs.rs/tree-sitter/latest/tree_sitter/)
- [Tree-sitter Elm grammar](https://github.com/elm-tooling/tree-sitter-elm)
- [Tree-sitter Haskell grammar](https://github.com/tree-sitter/tree-sitter-haskell)
- [Tree-sitter Python grammar](https://github.com/tree-sitter/tree-sitter-python)
- [Tree-sitter Rust grammar](https://github.com/tree-sitter/tree-sitter-rust)
- [`similar` diff crate](https://docs.rs/similar/latest/similar/)
- [`clap` derive reference](https://docs.rs/clap/latest/clap/_derive/)
- [`ratatui`](https://docs.rs/ratatui/latest/ratatui/)
- [`crossterm`](https://docs.rs/crossterm/latest/crossterm/)
- [`terminal-colorsaurus`](https://docs.rs/terminal-colorsaurus/latest/terminal_colorsaurus/)
- [`syntect`](https://docs.rs/syntect/latest/syntect/)
- [`two-face`](https://docs.rs/two-face/latest/two_face/)

Pin resolved versions in `Cargo.lock`. When upgrading `gix` or a grammar, read its changelog, inspect feature resolution or `NODE_TYPES`, and run the complete fixture and integration suite before accepting new snapshots.

## 23. Haskell projection

The Haskell adapter lives in `lang-haskell`. It owns the Haskell
grammar and every Haskell node name through `syntax.rs`.

### 23.1 General rules

- Parse the optional module header and retain the declared module name; omit the
  export list, exactly as Elm omits its exposing clause.
- Omit imports, comments, and Haddocks.
- Preserve pragmas (`{-# LANGUAGE ... #-}`, `{-# INLINE ... #-}`, and similar)
  because they can change a declaration's meaning. A pragma prefixes the
  declaration that follows it; a file-level pragma prefixes the module header.
- Consider only top-level declarations and declarations inside a class or
  instance body. Bindings inside `where` or `let` are never inspected.
- Preserve declaration order after omitted declarations are removed.

### 23.2 Types mode

Include:

- `data` and `newtype` declarations with type parameters, contexts, and kind
  annotations.
- Constructors in prefix, record, infix, and GADT form. A record renders one
  field per indented line; multiple constructors render one per indented line.
- `deriving` clauses with their strategies and `via` types.
- `type` synonyms.
- `type` and `data` families, including abstract and closed families with their
  equations, and `type`/`data` instances.
- `kind_signature` and `type_role` declarations.
- Class headers with contexts, type parameters, functional dependencies, and
  associated family declarations.
- Instance headers, including associated type and data instances.
- Standalone `deriving instance` declarations.

Canonical examples:

```haskell
data User
    = User UserId (Maybe Email)
    | Anonymous
    deriving (Eq, Show)

data User = User
    { name :: Text
    , email :: Email
    }
    deriving (Eq, Show)

class Eq a => Container f a where
    empty :: f a
```

A class or instance in Types mode shows its header only; the `where` keyword is
emitted only when a member is actually displayed, so a Types-mode class reads as
`class Eq a => Container f a`.

### 23.3 Signatures mode

Include everything from Types mode plus:

- Top-level type signatures, including multi-name signatures (`baz, qux :: Int`).
- Top-level function and binding declarations that have no matching explicit
  signature, rendered as their written head (`bar x`, `hidden`). A definition
  whose name is covered by a signature is represented by the signature alone,
  regardless of source order.
- Class method signatures and default-method signatures.
- Instance method heads, with their written argument patterns and no invented
  types; a method definition already covered by a signature is not repeated.
- Foreign imports.
- Pattern-synonym signatures (`pattern Single :: a -> [a]`).

Function, method, and binding bodies are never rendered, so a body-only edit
leaves both projections unchanged.

## 24. Python projection

The Python adapter lives in `lang-python`. It owns the Python grammar
and every Python node name through `syntax.rs`.

### 24.1 General rules

- Use the repository-relative file path as the outer file context; Python has
  no module declaration.
- Omit imports, comments, and docstrings.
- Preserve decorators because they can change a declaration's meaning
  (`@dataclass`, `@property`, `@staticmethod`, `@overload`, and similar).
- Consider only module- and class-level declarations. Functions, classes,
  lambdas, and comprehensions inside a function body are never inspected.
- Preserve declaration order after omitted declarations are removed.

### 24.2 Types mode

Include:

- `class` declarations with base classes, keyword arguments, and PEP 695 type
  parameters.
- Class-level annotated attributes, rendered as fields.
- Enum members of a class whose bases name an enum type, rendered like
  constructors.
- PEP 695 `type` aliases.
- `TypeVar`, `ParamSpec`, `TypeVarTuple`, and `NewType` declarations.

Canonical example:

```python
@dataclass(frozen=True)
class Point(Base, metaclass=Meta):
    x: float
    y: float
```

### 24.3 Signatures mode

Include everything from Types mode plus:

- `def` and `async def` signatures at module and class scope, with parameters,
  annotations, defaults, return types, decorators, and type parameters.
- Module-level and class-level values with declared types, with the initializer
  removed.
- Module-level and class-level values without a declared type, rendered as their
  bare name.
- Function and method bodies are replaced with the `...` placeholder.
- Unannotated declarations are rendered exactly as written; Python does not use
  Elm's missing-annotation placeholder.

Canonical examples:

```python
def load(id: UserId, *, source=None, **options) -> Profile | None: ...

@staticmethod
def parse(text: str) -> int: ...

MAX: int
```
