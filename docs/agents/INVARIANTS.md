# Invariants

Audience: agents and maintainers. These are the properties a change must not
break. Each maps to a test or check that guards it. `TECHNICAL_DESIGN.md` is
authoritative for the reasoning.

## Output determinism

- Projection and diff output does not depend on the current directory, locale,
  terminal width, wall clock, or environment variables.
- Show files and scope traversal use raw repository-path byte order. Diff events
  are keyed and sorted by target path, using the sole present path for additions
  and deletions.
- Fuzzy move candidates are ordered by score, then old and new raw path bytes;
  exact ambiguous candidates pair by old and new raw path bytes. Display
  escaping never affects pairing or order.
- A focused document ends with exactly one trailing newline; an empty result is
  empty.
- Repeated runs on the same inputs produce identical bytes.

Guarded by: end-to-end CLI tests and the fixture suites.

## Projection invariance

For every supported language, changing only the following leaves **both**
projections byte-for-byte unchanged:

- a function or method body;
- comments and documentation;
- whitespace and formatting.

Conversely:

- changing a type changes the Types and Signatures projections;
- changing a signature changes only the Signatures projection;
- adding a private function changes Signatures output;
- reordering declarations changes projected order.

The Tests projection is additionally invariant to non-test declarations: adding a
non-test function, or changing a type, leaves it byte-for-byte unchanged.

Guarded by: `crates/lang-*/tests/invariance.rs` against `fixtures/*/`.

## Diff invariant

If two snapshots have equal canonical projections, an unmoved file is omitted
and produces no diff. A body-only change therefore produces an empty focused
diff. The file-move case is the deliberate exception: a detected move is emitted
even when its projections are equal, so a body-only edit inside a move keeps the
move visible with no content hunks. A move whose two projections are both empty
in the requested mode is omitted. This move exception is part of focused
diffing, not a fallback to raw source.

Move detection runs once per comparison over the complete supported snapshots,
before path or area filtering, so a narrow scope cannot change the pairing. A
scope matches a move when either path matches; the CLI returns the entire event
and the TUI keys it under the target path. Same-path pairs take precedence, each
path participates in at most one move, and copies are not detected.

Guarded by: `crates/engine/tests/engine.rs` and CLI diff tests.

## Schema stability

- `codect.show.v1` and `codect.diff.v2` are the current schemas. `codect.diff.v1`
  is archived and is not emitted by the current binary.
- A consumer tolerates unknown fields; a `schema` value other than the expected
  one is a fatal, explicit version mismatch.
- Fields may be added additively within a version. Widening an enumerated value
  set is a compatibility change, not an additive field: `mode` gained `tests`
  and diff `status` gained `moved`, so a consumer validating either enum rejects
  the new value. That is intended — a Tests projection must not be read as full
  Signatures, and a move must not silently become a delete plus an add.
- In v2, a moved file has both sides and a distinct `old_path`; other statuses
  have no `old_path` and carry `equal: false`. The canonical equality of a
  move's projections is carried by `equal`.
- `codect.show.v1` may carry optional additive
  `path_encoding: "codect.path.v1"`; absence means legacy path display, and
  unknown values are unsupported for path decoding. `codect.diff.v2` always uses
  the fixed new encoding and needs no marker.
- `projection.text` is exactly what text mode emits, so the JSON and text views
  cannot drift.
- The Signatures projection is a superset of Types and of Tests by `stable_key`,
  so Signatures is the outline superset for every mode.

Guarded by: CLI golden documents under `fixtures/schema/`, the in-crate JSON
Schema validation in the CLI tests, and editor schema gates. The Neovim
Diffview adapter requires v2 and has pinned move suites.

## Dependency and architecture boundaries

- `base` does not depend on `gix`, Tree-sitter, any grammar, `clap`, or file
  formats. Language node names never appear in `base`.
- `gix` types never leave `git`.
- `engine` does not depend on `clap`, `miette`, `ratatui`, or `crossterm`.
- The terminal frontend is three crates: `tui-core` (kernel, plugin contract,
  design system), `tui` (built-in plugins and the registry), and `tui-diff`
  (the optional compare and history views).
- `tui-core` depends on no plugin crate and names no plugin: no plugin type,
  plugin id string, or plugin module appears in it. Guarded by
  `just check-tui-core-boundary` and the kernel boundary scan.
- `tui-diff` depends on `tui-core` only among the TUI crates; it never depends
  on `tui`.
- `tui` builds and passes its tests with `--no-default-features` (no diff), and
  the no-diff tree contains no `tui-diff`. Guarded by `just check-tui-nodiff`.
- None of the three depends on `clap` or `miette`, discovers repositories,
  parses arguments, or reads `.codect.toml`.
- Building `cli --no-default-features` links none of `ratatui`, `crossterm`,
  `terminal-colorsaurus`, `syntect`, or `two-face`.
- Only the approved `gix` features are enabled: `revision`, `dirwalk` (which
  brings `attributes` and `excludes`), `sha1`, `sha256`, `auto-chain-error`,
  and `pack-cache-lru-static`.

Guarded by: `just check-workspace-nodefault`, `just check-tui-nodiff`, `just
check-tui-core-boundary`, and the `cargo tree` audit in `just check`.

## Safety and robustness

- Repositories and source files are untrusted. Codect does not execute
  repository configuration, hooks, filters, attributes, macros, build scripts,
  compilers, or formatters, and does not follow repository symlinks. Untracked
  `:worktree` enumeration reads the repository's ignore stack but executes no
  filter, hook, or other repository command.
- Move detection may read off-scope supported sources and worktree metadata to
  make pairing scope-independent. Detection source access is best-effort:
  unknown size, more than 256 KiB, invalid UTF-8, or a read failure makes a
  candidate fuzzy-ineligible, not fatal. Exact matching still considers every
  candidate and all sizes using raw bytes where needed; an unreadable worktree
  candidate cannot match exactly. A source selected for projection still uses
  the existing fatal read/parse path, and unchanged same-path entries with
  identical blob IDs still avoid blob reads. Exact worktree identity is computed
  from the source bytes without requiring the file to fit in memory.
- Fuzzy-eligible raw source is captured once before pair scoring; pair scoring
  performs no IO, and selected worktree projections reuse captured fuzzy bytes.
  The raw fuzzy source is bounded at 128 MiB, separate from the 32 MiB
  exact-worktree cache; profiles, interner tables, and LCS/Myers working memory
  make this not a total process-memory bound.
- Safe frequency and LCS-length-derived byte upper bounds skip only pairs
  provably below 50%. The optional LCS scratch is capped at 32 MiB; exceeding
  that cap skips only the bound, never the pair. Prepared profiles use a logical
  budget of 8,000,000 unit IDs, 5,000,000 light IDs, 2,000,000 heavy entries,
  and 8 MiB charged interner keys; these are accounting limits, not physical
  allocation/RSS bounds. Budget exhaustion falls back to the unchanged
  byte-slice scorer and never makes a pair ineligible. Every surviving pair uses
  the final deterministic Myers score; no semantic cap, dependency, or thread
  is added.
- Worktree reads/hashes are per-file fingerprint-guarded, and a selected reread
  is rehashed against any recorded exact identity; a same-size mutation fails
  as `WorktreeRead`. The worktree is not atomic across files; this is not a
  proof against concurrent or malicious mutation.
- Config files are size-bounded; caches are bounded.
- Allocation and parser failures are reported, not panicked.
- `panic!`, `unwrap`, and `expect` are reserved for tests and statically
  guaranteed initialization.

Guarded by: `crates/cli/tests/hardening.rs`, adapter traversal structure, and
review.

## See also

- [CLI and JSON contract](CONTRACT.md).
- [Projection model](PROJECTION.md).
- [`docs/TECHNICAL_DESIGN.md`](../TECHNICAL_DESIGN.md) sections 3, 5.2, 16, 18.
