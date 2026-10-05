# AGENTS.md

Codect gives selectable focused views and diffs of a codebase. It strips
implementation bodies so you can read the shape of the code — declarations,
types, and signatures — without reading how it works. Four languages: Elm,
Haskell, Python, and Rust.

This file is the entry point for agents. It is intentionally short; the linked
documents hold the detail.

## Invoke it

```sh
codect show --mode <types|signatures> [--path PATH | --area AREA]... [REVISION]
codect diff --mode <types|signatures> [--path PATH | --area AREA]... BASE TARGET

codect show --format json --mode types               # machine-readable projection
codect show --format json --mode types --stdin --path src/lib.rs
codect diff --format json --mode signatures HEAD :worktree
```

- `show` defaults `REVISION` to `HEAD`. `--mode` is required.
- Text output is the canonical projection, byte-for-byte stable.
- The snapshots `:index`, `:worktree`, `:empty`, and the empty-tree object ID
  are valid `diff` sides in both `text` and `json`. `:worktree` includes
  untracked, non-ignored files.
- `--path` and `--area` are mutually exclusive. Both are repeatable and union.

## Rely on

- Exit `0` success, `1` fatal (empty stdout, one diagnostic on stderr), `2`
  usage error.
- Read-only: Codect never writes the repository, worktree, or index, never opens
  an editor, and never executes repository code.
- JSON is versioned. Tolerate unknown fields; treat any other `schema` value as
  a fatal version mismatch.
- A body-only change produces an empty focused diff. That is intentional.
- `types`/`signatures` never include function bodies; never fall back to raw
  source.

## Read next

| Document | Contents |
| --- | --- |
| [CLI and JSON contract](docs/agents/CONTRACT.md) | Exact flags, exit codes, `codect.show.v1`, `codect.diff.v1`, snapshots |
| [Projection model](docs/agents/PROJECTION.md) | Modes, per-language rules, stable keys, outline/superset rule |
| [Invariants](docs/agents/INVARIANTS.md) | Determinism, invariance, schema stability, boundaries |
| [Product](docs/PRODUCT.md) | Authoritative behavior and scope |
| [Technical design](docs/TECHNICAL_DESIGN.md) | Full implementation contract |
| [Schemas](docs/schema/) | `codect.show.v1.json`, `codect.diff.v1.json` |
| [Plans](docs/plans/) | Self-contained work items / task packets |

## Changing the code

Run `just check` before claiming a change is done. It runs formatting, clippy,
the workspace tests, a release build, the `gix` feature audit, and the
`--no-default-features` build. See
[CONTRIBUTING.md](docs/CONTRIBUTING.md) for the full list and the test layout.
