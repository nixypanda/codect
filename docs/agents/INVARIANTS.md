# Invariants

Audience: agents and maintainers. These are the properties a change must not
break. Each maps to a test or check that guards it. `TECHNICAL_DESIGN.md` is
authoritative for the reasoning.

## Output determinism

- Projection and diff output does not depend on the current directory, locale,
  terminal width, wall clock, or environment variables.
- Files are processed in raw repository-path byte order.
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

If two snapshots have equal canonical projections, the file is omitted and
produces no diff. A body-only change therefore produces an empty focused diff.
This is the defining invariant of focused diffing.

Guarded by: `crates/engine/tests/engine.rs` and CLI diff tests.

## Schema stability

- `codect.show.v1` and `codect.diff.v1` are versioned by `schema`.
- A consumer tolerates unknown fields; a `schema` value other than the expected
  one is a fatal, explicit version mismatch.
- Fields may be added additively within a version. Widening an enumerated value
  set is a compatibility change, not an additive field: `mode` gained `tests`,
  so a consumer validating `mode` against the enum rejects it. That is intended —
  a Tests projection must not be read as full Signatures.
- `projection.text` is exactly what text mode emits, so the JSON and text views
  cannot drift.
- The Signatures projection is a superset of Types and of Tests by `stable_key`,
  so Signatures is the outline superset for every mode.

Guarded by: `editors/nvim/tests/test_schema.lua` and CLI golden documents under
`fixtures/schema/`, compared against the committed schemas.

## Dependency and architecture boundaries

- `base` does not depend on `gix`, Tree-sitter, any grammar, `clap`, or file
  formats. Language node names never appear in `base`.
- `gix` types never leave `git`.
- `engine` does not depend on `clap`, `miette`, `ratatui`, or `crossterm`.
- `tui` does not depend on `clap` or `miette`, discover repositories, parse
  arguments, or read `.codect.toml`.
- Building `cli --no-default-features` links none of `ratatui`, `crossterm`,
  `terminal-colorsaurus`, `syntect`, or `two-face`.
- Only the approved `gix` features are enabled: `revision`, `dirwalk` (which
  brings `attributes` and `excludes`), `sha1`, `sha256`, `auto-chain-error`,
  and `pack-cache-lru-static`.

Guarded by: `just check-workspace-nodefault` and the `cargo tree` audit in
`just check`.

## Safety and robustness

- Repositories and source files are untrusted. Codect does not execute
  repository configuration, hooks, filters, attributes, macros, build scripts,
  compilers, or formatters, and does not follow repository symlinks. Untracked
  `:worktree` enumeration reads the repository's ignore stack but executes no
  filter, hook, or other repository command.
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
