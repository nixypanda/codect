/**
 * Shared JSON schema types for the codect DSH plugin.
 *
 * These mirror the codect.show.v1 and codect.diff.v1 contracts that the
 * `codect` CLI emits with `--format json`. The backend runner parses these
 * directly; the frontend renders them.
 *
 * See docs/agents/CONTRACT.md in the codect repository for the canonical
 * projection model.
 */

// ---------------------------------------------------------------------------
// codect.show.v1
// ---------------------------------------------------------------------------

export type CodectShowSchema = "codect.show.v1";
/**
 * Which input produced a `show` document. `show` supports committed revisions
 * plus the editor source forms (`stdin` / `worktree`); unlike `diff` it has no
 * `:index` / `:worktree` / `:empty` snapshot sides.
 */
export type CodectInput = "revision" | "stdin" | "worktree";
export type CodectMode = "types" | "signatures";
/** The requested revision, or `null` for the editor source forms. */
export type CodectRevision = string | null;

/** A source location within a file. */
export interface SourceSpan {
  start_line: number;
  end_line: number;
  start_byte: number;
  end_byte: number;
}

/**
 * One projected declaration: a type, signature, variant, field, etc.
 * `null` `parent_key` means a top-level declaration.
 */
export interface ShowItem {
  stable_key: string;
  parent_key: string | null;
  kind: string;
  name: string;
  span: SourceSpan;
  canonical_text: string;
}

/** An outline item — mode-independent, signature-level projection. */
export interface OutlineItem {
  stable_key: string;
  parent_key: string | null;
  kind: string;
  name: string;
  span: SourceSpan;
  signature: string;
  /** True when the requested mode's projection retains this stable_key. */
  retained_in_mode?: boolean;
}

/** The projection of a single file. */
export interface ShowFile {
  path: string;
  language: string;
  projection: {
    text: string;
    items: ShowItem[];
  };
  /** Mode-independent outline — signatures without bodies. */
  outline: OutlineItem[];
}

/** The full `codect show --format json` document. */
export interface ShowDocument {
  schema: CodectShowSchema;
  input: CodectInput;
  revision: CodectRevision;
  mode: CodectMode;
  files: ShowFile[];
}

// ---------------------------------------------------------------------------
// codect.diff.v1
// ---------------------------------------------------------------------------

export type CodectDiffSchema = "codect.diff.v1";

/** A revision point of a diff: which input kind, its revision text, and its id. */
export interface DiffRevision {
  kind: "commit" | "worktree" | "index" | "empty";
  revision: string;
  id: string; // full commit id (empty for non-commit sides)
}

/**
 * One side of a changed file: its projection and mode-independent outline at
 * that revision. Mirrors the `side` definition in codect.diff.v1.
 */
export interface DiffFileSide {
  snapshot_id: string;
  projection: {
    text: string;
    items: ShowItem[];
  };
  outline: OutlineItem[];
}

/** Status of a single file in the diff. Equal projections are omitted entirely. */
export type DiffFileStatus = "added" | "deleted" | "modified";

/**
 * A single file's diff. An absent side is `null` (added files have no `base`,
 * deleted files have no `target`). `equal` is always `false`; unchanged files
 * are not present.
 */
export interface DiffFile {
  path: string;
  language: string;
  status: DiffFileStatus;
  base: DiffFileSide | null;
  target: DiffFileSide | null;
  equal: false;
}

/** The full `codect diff --format json` document. */
export interface DiffDocument {
  schema: CodectDiffSchema;
  mode: CodectMode;
  base: DiffRevision;
  target: DiffRevision;
  files: DiffFile[];
}

/**
 * A declaration-level change derived on the client by comparing the two sides
 * of a `DiffFile` outline. `base_text` / `target_text` carry the outline
 * signature of the side they describe.
 */
export interface DiffChange {
  status: "added" | "deleted" | "changed";
  item: OutlineItem;
  base_text: string | null;
  target_text: string | null;
}

// ---------------------------------------------------------------------------
// Common
// ---------------------------------------------------------------------------

export type CodectDocument = ShowDocument | DiffDocument;

/** Exit codes from the codect CLI. */
export const CODEXT_EXIT = {
  SUCCESS: 0,
  FATAL: 1, // empty stdout, one diagnostic on stderr
  USAGE: 2, // usage error
} as const;

export type CodectExitCode = typeof CODEXT_EXIT[keyof typeof CODEXT_EXIT];
