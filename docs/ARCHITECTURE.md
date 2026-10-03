# Architecture

Codect is a Cargo workspace. This page is a map; the authoritative contract is
[`TECHNICAL_DESIGN.md`](TECHNICAL_DESIGN.md).

## Crates

| Crate | Role |
| --- | --- |
| `base` | The grammar-agnostic core: projection model, repository paths, stable keys, canonical layout document, diff model, outline model. No Git, no parsers, no CLI. |
| `git` | Read-only repository access through `gix`: discovery, revision resolution, tree traversal, stage-zero index reads, blob reads. `gix` types never leave this crate. |
| `lang-elm`, `lang-haskell`, `lang-python`, `lang-rust` | One adapter per language: Tree-sitter parsing, extraction into the shared model, and canonical rendering. All grammar node names live here. |
| `engine` | The shared Git-aware application layer. Composes `git` reads with `base` rendering and exposes `show` and `diff` over a selection. No CLI, terminal, or rendering. |
| `tui` | The optional terminal frontend. Owns the TEA loop, terminal lifecycle, and all UI. |
| `cli` | Argument parsing, engine invocation, output rendering, diagnostics, exit codes, and JSON serialization. |

## Dependency direction

```text
cli
  ├── base
  ├── git
  ├── engine
  └── tui (optional, default-on)

tui              ──→ base, engine
engine           ──→ base, git, lang-elm, lang-haskell, lang-python, lang-rust
lang-*           ──→ base
git              ──→ base
```

The boundaries are enforced by review and by the required checks:

- `base` must not depend on `gix`, Tree-sitter, a grammar, `clap`, or any file
  format. Language-specific node names never appear in `base`.
- `engine` must not depend on `clap`, `miette`, `ratatui`, or `crossterm`.
- `tui` must not depend on `clap` or `miette`, discover repositories, parse
  arguments, or read `.codect.toml`.
- Building `cli --no-default-features` links no terminal dependency.

Inside `tui` the graph is one-way and bottom-up:
`render`/`util` → `component` → `page` → `view`/`app` → `lib`.

## Pipeline

The pipeline lives once in `engine` and is shared by the CLI and the TUI.

`show`:

```text
repository path + revision + selection
  → discover repository → resolve revision → traverse tree
  → select supported blobs → reject absent selected paths
  → apply path scope → read blobs → project per language
  → sort by raw path bytes → render file sections
```

`diff`:

```text
base + target + selection
  → resolve both → traverse both → union supported paths
  → reject selected paths absent from both
  → for each path in byte order:
      outside scope or unchanged blob → skip
      one side only                   → compare with empty
      both sides                      → project both; compare canonical text
  → emit only files whose projections differ
```

A source change that produces the same projection produces no output. That is
the defining invariant of focused diffing.

## Core ideas

- **Projection model.** A projection is a list of `ProjectedItem`s with stable
  keys and spans, plus canonical text derived from them. It is not a universal
  AST.
- **Canonical text.** Rendering is deterministic and independent of original
  formatting and comments, so formatting-only edits disappear from focused
  diffs. Line breaks are fixed by declaration shape and an 80-column budget.
- **Language adapters.** Core selects an adapter by path, invokes it, renders
  file framing, and compares canonical text. Adapters own all language meaning.
- **Read-only.** Every path reads committed blobs, the tracked worktree, the
  index, or standard input, and never writes.

## Where to read more

| Topic | Document |
| --- | --- |
| Workspace layout, dependencies, boundaries | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) sections 2–4 |
| Core model, paths, stable keys, scoping | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) sections 5–6 |
| Pipeline and Git layer | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) sections 7–9 |
| Rendering, diff engine, CLI | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) sections 10–15 |
| Per-language projection | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) sections 11–12, 23–24 |
| Terminal frontend | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) section 21 |
| Testing and checks | [CONTRIBUTING.md](CONTRIBUTING.md) |
