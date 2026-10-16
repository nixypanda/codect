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

export type CodectShowSchema = "codect.show.v1";
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
  files: ShowFile[];
}

export type CodectDiffSchema = "codect.diff.v1";

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

export type DiffFileStatus = "added" | "deleted" | "modified";

export interface DiffFile {
  path: string;
  language: string;
  status: DiffFileStatus;
  base: DiffFileSide | null;
  target: DiffFileSide | null;
  equal: false;
}

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
