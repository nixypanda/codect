/**
 * Shared JSON schema types for the codect DSH plugin.
 *
 * These mirror the codect.show.v1 and codect.diff.v2 contracts that the
 * `codect` CLI emits with `--format json`. The backend runner parses these
 * directly; the frontend renders them. Show stays on v1; only diff moved to
 * v2 for file-move events.
 *
 * See docs/agents/CONTRACT.md in the codect repository for the canonical
 * projection model.
 */

export type CodectShowSchema = "codect.show.v1";
export type CodectPathEncoding = "codect.path.v1";
export type CodectInput = "revision" | "stdin" | "worktree";
export type CodectMode = "types" | "signatures";
export type CodectRevision = string | null;

export interface SourceSpan {
  start_line: number;
  end_line: number;
  start_byte: number;
  end_byte: number;
}

export interface ShowItem {
  stable_key: string;
  parent_key: string | null;
  kind: string;
  name: string;
  span: SourceSpan;
  canonical_text: string;
}

export interface OutlineItem {
  stable_key: string;
  parent_key: string | null;
  kind: string;
  name: string;
  span: SourceSpan;
  signature: string;
  retained_in_mode?: boolean;
}

export interface ShowFile {
  path: string;
  language: string;
  projection: {
    text: string;
    items: ShowItem[];
  };
  outline: OutlineItem[];
}

export interface ShowDocument {
  schema: CodectShowSchema;
  input: CodectInput;
  revision: CodectRevision;
  mode: CodectMode;
  /**
   * Additive marker for the injective path display encoding. Absent is the
   * legacy show encoding: do not interpret backslash escapes in `path`.
   */
  path_encoding?: CodectPathEncoding;
  files: ShowFile[];
}

export type CodectDiffSchema = "codect.diff.v2";

export interface DiffRevision {
  kind: "commit" | "worktree" | "index" | "empty";
  revision: string;
  id: string;
}

export interface DiffFileSide {
  snapshot_id: string;
  projection: {
    text: string;
    items: ShowItem[];
  };
  outline: OutlineItem[];
}

export type DiffFileStatus = "added" | "deleted" | "modified" | "moved";

interface DiffFileBase {
  /** Target-side display/selection key; for a deletion, the removed path. */
  path: string;
  language: string;
  base: DiffFileSide | null;
  target: DiffFileSide | null;
  equal: boolean;
}

export interface AddedDiffFile extends DiffFileBase {
  status: "added";
  old_path?: never;
  base: null;
  target: DiffFileSide;
  equal: false;
}

export interface DeletedDiffFile extends DiffFileBase {
  status: "deleted";
  old_path?: never;
  base: DiffFileSide;
  target: null;
  equal: false;
}

export interface ModifiedDiffFile extends DiffFileBase {
  status: "modified";
  old_path?: never;
  base: DiffFileSide;
  target: DiffFileSide;
  equal: false;
}

/**
 * A move has both projections. `path` is the target path and `old_path` the
 * source path; they are always distinct. `equal` says whether the two canonical
 * projection texts match, so an exact or body-only move is `true` and a move
 * with a projected change is `false`.
 */
export interface MovedDiffFile extends DiffFileBase {
  status: "moved";
  old_path: string;
  base: DiffFileSide;
  target: DiffFileSide;
  equal: boolean;
}

export type DiffFile =
  | AddedDiffFile
  | DeletedDiffFile
  | ModifiedDiffFile
  | MovedDiffFile;

export interface DiffDocument {
  schema: CodectDiffSchema;
  mode: CodectMode;
  base: DiffRevision;
  target: DiffRevision;
  files: DiffFile[];
}

export interface DiffChange {
  status: "added" | "deleted" | "changed";
  item: OutlineItem;
  base_text: string | null;
  target_text: string | null;
}

export type CodectDocument = ShowDocument | DiffDocument;

export const CODEXT_EXIT = {
  SUCCESS: 0,
  FATAL: 1, // empty stdout, one diagnostic on stderr
  USAGE: 2, // usage error
} as const;

export type CodectExitCode = typeof CODEXT_EXIT[keyof typeof CODEXT_EXIT];
