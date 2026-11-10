# CLI and JSON contract

Audience: agents and other machine consumers. This is the stable interface of the
`codect` binary.

`PRODUCT.md` is authoritative for behavior and scope. `TECHNICAL_DESIGN.md`
sections 14 and 15 are authoritative for the full contract. This page is the
short version a caller can depend on.

## Invocation

```text
codect show --format <text|json> --mode <types|signatures|tests> [--path <PATH> | --area <AREA>]... [REVISION]
codect diff --format <text|json> --mode <types|signatures|tests> [--merge-base] [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`tests` is recognized for Rust and Python only. Elm and Haskell retain nothing
in that mode, including a module header.

- `show` with an omitted `REVISION` projects supported files on disk (Git-independent); an explicit `REVISION` requires Git.
- `--mode` is required; there is no default.
- `tests` is the Signatures projection restricted to test declarations, and is
  recognized for Rust and Python only. An Elm or Haskell file retains nothing,
  including its module header, so it projects to an empty declaration list. An
  empty projection therefore does not distinguish "detection unavailable" from
  "no tests"; `PRODUCT.md` records that as a known gap.
- `--format` defaults to `text`. `text` output is the canonical projection and is
  byte-for-byte stable.
- `--format json` on `diff` additionally accepts the snapshots `:index`,
  `:worktree`, `:empty`, and the canonical Git empty-tree object ID as a side.
  The `text` format accepts the same sides.
- `--path`/`-p` and `--area`/`-a` are mutually exclusive (usage error, exit `2`).
  Each is repeatable and forms a union. `--area` paths are repository-root
  relative; `--path` resolves relative to the current directory.
- `--color <auto|always|never>` defaults to `auto`. JSON is always uncolored
  and never contains ANSI.
- On `diff`, `--merge-base` replaces `<BASE>` with the merge base of `<BASE>`
  and `<TARGET>` (the equivalent of `git diff BASE...TARGET`), excluding
  commits that only landed on `<BASE>` after the branches diverged. Both sides
  must be commits: a snapshot side is a usage error (exit `2`), and two commits
  with no common ancestor exit `1`.

## Editor input forms

```text
codect show --format json --mode <types|signatures|tests> --stdin    --path <FILE>
codect show --format json --mode <types|signatures|tests> --worktree --path <FILE>
```

- Both require exactly one `--path` naming a **file** (a directory is exit `2`)
  and are exclusive with `REVISION` and `--area`.
- `--stdin` reads the buffer bytes; `--worktree` reads the file from disk.
- `--worktree` refuses a symlinked target and a resolved path that leaves the
  root (the repository worktree when inside Git, otherwise the current
  directory). Both `--stdin` and `--worktree` operate outside Git.
- These inputs are read-only; all Codect paths are read-only.
## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. Includes a diff with focused changes and a selection that exists but contains no supported files (empty output). |
| `1` | Fatal: no repository or worktree when one is required (explicit revision, diff, or in-Git disk show); bad revision; tree/index/snapshot enumeration; Git object read required for the operation; absent selected path; config/area failure; or a source selected for projection that is non-UTF-8 or fails to parse. Move-detection discovery reads remain per-file best effort. Empty stdout, one diagnostic on stderr. |
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
projections are equal are omitted, except for a detected move with at least one
nonempty requested-mode projection: such a move is emitted even when both
projections are equal, and then has no content headers or hunks. A pure move is
rendered as:

```text
diff --codect a/src/old.rs b/src/new.rs
rename from src/old.rs
rename to src/new.rs
```

An edited move follows the two `rename` lines with `--- a/<old>`,
`+++ b/<new>`, and ordinary focused `@@` hunks. A move whose two projections are
both empty in the requested mode is omitted. A non-empty document ends with
exactly one trailing newline; an empty document is empty.

## Path display encoding

Text headers from the current binary carry Codect's injective display encoding.
`codect.diff.v2` uses it for JSON `path`/`old_path`. `codect.show.v1` uses it for
JSON `path` only when the optional top-level field
`path_encoding: "codect.path.v1"` is present:

- Ordinary valid Unicode is unchanged.
- A literal backslash is written `\\`.
- LF, CR, and TAB are written `\n`, `\r`, and `\t`.
- Every other C0 byte and DEL is written `\xNN`, with two uppercase hex digits.
- Every invalid UTF-8 byte is written `\xNN`.

The encoding doubles literal backslashes and escapes ASCII C0 control bytes,
DEL, and invalid UTF-8 bytes, so those bytes cannot inject a header line or
terminal control sequence. Other valid Unicode, including C1 control characters,
is emitted unchanged; this is not a general Unicode terminal-control sanitizer.
Processing, pairing, and sorting use raw repository-path bytes; display encoding
never changes order or identity.

JSON adds normal JSON string escaping on top of this encoding. For example, the
two-character display form `\n` is serialized as `\\n` in JSON.

For `codect.diff.v2`, JSON-decode first, then decode the Codect display encoding
to recover raw path bytes before a raw-path host API lookup. For
`codect.show.v1`, do this only when `path_encoding: "codect.path.v1"` is present.
Absence means legacy show display semantics; never decode an absent-marker show
path with the `codect.path.v1` decoder. An unknown `path_encoding` value is
unsupported for path decoding and should be rejected gracefully rather than
guessed. Do not treat a JSON `path` value as raw and unescaped.

The `codect.show.v1` schema remains `codect.show.v1`; its required fields are
unchanged and it gains the optional additive `path_encoding` property.
`codect.diff.v2` always uses the fixed new encoding and needs no marker.

## JSON documents

Both JSON documents are versioned by a `schema` field. **Consumers must tolerate
unknown fields and must treat any other `schema` value as a fatal, explicit
version mismatch.** Additive fields may appear within the same version.

### `codect.show.v1`

Schema: [`docs/schema/codect.show.v1.json`](../schema/codect.show.v1.json).

Root: `schema`, `input` (`revision` | `stdin` | `worktree`), `revision`
(string or null), `mode`, and `files`. The new producer also emits the optional
additive `path_encoding: "codect.path.v1"`. When absent, `path` has legacy
display semantics; when present with that value, it uses the section above.
Unknown values are unsupported for path decoding. A filesystem projection from an
omitted revision uses `input: "worktree"` and `revision: null`.
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

### `codect.diff.v2`

Schema: [`docs/schema/codect.diff.v2.json`](../schema/codect.diff.v2.json).
`codect.diff.v1` is archived and is not emitted by the current binary. This is a
version change, not a silent enum widening: v1 enumerated only added, deleted,
and modified and required `equal: false`.

Root: `schema`, `mode`, `base`, `target`, `files`.

- Each snapshot has a `kind` (`commit` | `index` | `worktree` | `empty`), the
  requested `revision`, and an `id`.
- A `commit` ID is resolved and stable. `:index` and `:worktree` IDs are
  **mutable labels**, not content hashes: refresh after staging or disk writes.
- Under `--merge-base`, the base snapshot carries `merge_base: true`; its
  `revision` stays the requested base and its `id` is the resolved merge base.
  The field is additive and omitted otherwise.
- Each changed file has `path`, `language`, `status` (`added` | `deleted` |
  `modified` | `moved`), `base`/`target` sides, and `equal`. An absent side is
  `null`; a present side has `snapshot_id`, `projection`, and `outline`.
- `added` has a null base and a target side; `deleted` has a base side and a
  null target; `modified` has both sides. These statuses always carry
  `equal: false` and omit `old_path`.
- For a moved file, `path` is the target path, `old_path` is the source path,
  both sides are present, and `equal` is `true` exactly when the two canonical
  projection texts match. The producer and tests enforce that `old_path` and
  `path` are distinct; JSON Schema cannot compare them.
- A move whose two requested-mode projections are both empty is omitted.
- A move matches a path or area selection when either its old or new path
  matches, and the whole event is returned. Detection still runs over the
  complete supported snapshots.
- Move detection may inspect off-scope supported sources so scope cannot change
  pairing. Detection access is best-effort: unknown size, more than 256 KiB,
  invalid UTF-8, or a read failure makes a candidate fuzzy-ineligible, not
  fatal. Exact matching still considers every candidate and all sizes using raw
  bytes where needed; an unreadable worktree candidate cannot match exactly. A
  source selected for projection still uses the normal fatal read, UTF-8, and
  parse behavior.
- Fuzzy move pairing is a source-content heuristic, not historical identity:
  unrelated files with shared boilerplate can pair. Exact content matching runs
  first, and the deterministic score plus raw-path ordering governs greedy
  selection.
- Detection reads raw source bytes without Git filters, end-of-line conversion,
  or checkout normalization. An LF versus CRLF difference can prevent a pair.
- Fuzzy-eligible source bytes are captured once before pair scoring; pair
  scoring performs no source IO. Selected worktree projections reuse captured
  fuzzy bytes. The captured raw fuzzy source is bounded at 128 MiB and is
  separate from a 32 MiB exact-worktree cache; profiles, interner tables,
  LCS/Myers work, and other process memory are outside that bound.
- Safe frequency and LCS-length-derived byte upper bounds skip only pairs
  provably below 50%; surviving pairs use the final deterministic Myers score.
  No semantic cap, dependency, or thread is introduced.
- Worktree reads and hashes are guarded by before/after file fingerprints
  (length, mtime, and on Unix device/inode/ctime/mtime nanoseconds). If a
  selected worktree source must be reread, it is rehashed against any recorded
  exact identity; a same-size mutation is a normal `WorktreeRead` failure. The
  worktree view is not atomic across files, so this is not a race-freedom claim
  against concurrent or malicious mutation.
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
