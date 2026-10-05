# CLI and JSON contract

Audience: agents and other machine consumers. This is the stable interface of the
`codect` binary.

`PRODUCT.md` is authoritative for behavior and scope. `TECHNICAL_DESIGN.md`
sections 14 and 15 are authoritative for the full contract. This page is the
short version a caller can depend on.

## Invocation

```text
codect show --format <text|json> --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
codect diff --format <text|json> --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

- `show` defaults `REVISION` to `HEAD`.
- `--mode` is required; there is no default.
- `--format` defaults to `text`. `text` output is the canonical projection and is
  byte-for-byte stable.
- `--format json` on `diff` additionally accepts the snapshots `:index`,
  `:worktree`, `:empty`, and the canonical Git empty-tree object ID as a side.
  The `text` format accepts the same sides.
- `--path`/`-p` and `--area`/`-a` are mutually exclusive (usage error, exit `2`).
  Each is repeatable and forms a union. `--area` paths are repository-root
  relative; `--path` resolves relative to the current directory.
- `--color <auto|always|never>` defaults to `auto`. JSON is always raw.

## Editor input forms

```text
codect show --format json --mode <types|signatures> --stdin    --path <FILE>
codect show --format json --mode <types|signatures> --worktree --path <FILE>
```

- Both require exactly one `--path` naming a **file** (a directory is exit `2`)
  and are exclusive with `REVISION` and `--area`.
- `--stdin` reads the buffer bytes; `--worktree` reads the file from disk.
- `--worktree` refuses a symlinked target and a resolved path that leaves the
  repository.
- These inputs are read-only; all Codect paths are read-only.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. Includes a diff with focused changes and a selection that exists but contains no supported files (empty output). |
| `1` | Fatal: no repository, bad revision, absent selected path, config/area failure, non-UTF-8 source, parse failure. Empty stdout, one diagnostic on stderr. |
| `2` | Usage error, raised through `clap`. |

A selected path that names nothing in the projected revision (or in either side
of a diff) is fatal. A path that exists but contains no supported files
succeeds with empty output. A missing diagnostic is never acceptable; the tool
never silently falls back to raw source and never emits a partial diff after a
projection error.

## Text output framing

```text
== src/User.elm ==
<canonical projection>

== src/Session.rs ==
<canonical projection>
```

Diff uses Git-style per-file headers:

```text
diff --codect a/src/User.elm b/src/User.elm
--- a/src/User.elm
+++ b/src/User.elm
```

The absent side of an added or deleted file is `/dev/null`. Files whose
projections are equal are omitted. A non-empty document ends with exactly one
trailing newline; an empty document is empty.

## JSON documents

Both JSON documents are versioned by a `schema` field. **Consumers must tolerate
unknown fields and must treat any other `schema` value as a fatal, explicit
version mismatch.** Additive fields may appear within the same version.

### `codect.show.v1`

Schema: [`docs/schema/codect.show.v1.json`](../schema/codect.show.v1.json).

Root: `schema`, `input` (`revision` | `stdin` | `worktree`), `revision`
(string or null), `mode`, and `files`.

Each file carries:

- `path`, `language`.
- `projection`: `text` (exactly what text mode emits for the file) and `items`
  (`stable_key`, `parent_key`, `kind`, `name`, `span`, `canonical_text`).
- `outline`: a mode-independent, complete declaration list derived from the
  Signatures superset. Each entry has `stable_key`, `parent_key`, `kind`,
  `name`, `span`, `signature`, and `retained_in_mode`.

How to use the outline:

- When `retained_in_mode` is true, the mode-correct closed-fold text is the
  matching `projection.items[].canonical_text`.
- When it is false, use `signature` (the Signatures superset form). The two
  differ for containers such as a trait implementation whose Types form omits
  members the Signatures form shows.
- Line numbers are one-based on the wire; byte offsets are zero-based into the
  decoded UTF-8 source.
- `span` starts at the declaration node, so preceding attributes, decorators,
  pragmas, and doc comments are excluded. Extend a fold start upward over those
  lines.
- `stable_key` is unique within a file, not across the repository. Key global
  state by `(path, stable_key)`, not by `stable_key` alone.

### `codect.diff.v1`

Schema: [`docs/schema/codect.diff.v1.json`](../schema/codect.diff.v1.json).

Root: `schema`, `mode`, `base`, `target`, `files`.

- Each snapshot has a `kind` (`commit` | `index` | `worktree` | `empty`), the
  requested `revision`, and an `id`.
- A `commit` ID is resolved and stable. `:index` and `:worktree` IDs are
  **mutable labels**, not content hashes: refresh after staging or disk writes.
- Each changed file has `path`, `language`, `status` (`added` | `deleted` |
  `modified`), `base`/`target` sides, and `equal: false`. An absent side is
  `null`; a present side has `snapshot_id`, `projection`, and `outline`.
- `:index` reads stage-zero index blobs. `:worktree` reads tracked regular files
  on disk plus untracked, non-ignored files, and does **not** include unsaved
  editor buffers.

## Read-only guarantee

Codect reads committed blobs, the tracked worktree, the index, and standard
input. It never writes the repository, worktree, or index. It never opens an
editor and never executes repository code.

## See also

- [Projection model](PROJECTION.md) — modes, per-language rules, stable keys.
- [Invariants](INVARIANTS.md) — what may not change.
- [`docs/PRODUCT.md`](../PRODUCT.md) — behavior and scope.
- [`docs/TECHNICAL_DESIGN.md`](../TECHNICAL_DESIGN.md) — sections 14–15.
