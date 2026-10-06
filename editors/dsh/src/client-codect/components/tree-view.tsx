/**
 * tree-view.tsx — A foldable file tree for the codect sidebar views.
 *
 * The tree model is built by the pure `tree.ts`; this renders it with the DSH
 * theme tokens. No reusable file-tree component is exposed to plugins, so the
 * rows are drawn here (the shipped `sidebar-files` tree was the reference).
 *
 * Rows use the shared `FileTypeIcon` / folder glyphs when the primitives are
 * available and fall back to the plain `▾`/`▸` twisty otherwise.
 */

import React from "react";
import type { TreeRow } from "../tree";
import {
  FileTypeIcon,
  classifyFileType,
  IconFolderCloseRegular,
  IconFolderOpenRegular,
} from "../primitives";

export type FileStatus = "added" | "deleted" | "modified";

export interface TreeViewProps {
  rows: TreeRow[];
  selectedPath?: string | null;
  onSelect: (path: string) => void;
  onToggleDir: (path: string) => void;
  statusOf?: (path: string) => FileStatus | undefined;
}

const BADGE_LETTER: Record<FileStatus, string> = {
  added: "A",
  deleted: "D",
  modified: "M",
};

const hasFolderIcons =
  typeof IconFolderOpenRegular === "function" &&
  typeof IconFolderCloseRegular === "function";
const hasFileIcon =
  typeof FileTypeIcon === "function" && typeof classifyFileType === "function";

function RowIcon({ row }: { row: TreeRow }) {
  if (row.kind === "dir") {
    if (hasFolderIcons) {
      return (
        <span className="codect-tree-icon" aria-hidden="true">
          {row.expanded ? (
            <IconFolderOpenRegular size={16} />
          ) : (
            <IconFolderCloseRegular size={16} />
          )}
        </span>
      );
    }
    return (
      <span className="codect-twisty" aria-hidden="true">
        {row.expanded ? "▾" : "▸"}
      </span>
    );
  }

  if (hasFileIcon) {
    return (
      <span className="codect-tree-icon" aria-hidden="true">
        <FileTypeIcon kind={classifyFileType(row.label)} size={16} />
      </span>
    );
  }
  return <span className="codect-twisty" aria-hidden="true" />;
}

export function TreeView({
  rows,
  selectedPath,
  onSelect,
  onToggleDir,
  statusOf,
}: TreeViewProps) {
  return (
    <div className="codect-tree">
      {rows.map((row) => {
        const selected = row.kind === "file" && row.path === selectedPath;
        const status = row.kind === "file" ? statusOf?.(row.path) : undefined;
        return (
          <div
            key={`${row.kind}:${row.path}`}
            className={`codect-row${selected ? " codect-row-selected" : ""}${
              row.kind === "dir" ? " codect-row-dir" : ""
            }`}
            style={{ paddingLeft: 10 + row.depth * 18 }}
            role="treeitem"
            aria-selected={selected}
            aria-expanded={row.kind === "dir" ? row.expanded : undefined}
            title={row.path}
            onClick={() => {
              if (row.kind === "dir") onToggleDir(row.path);
              else onSelect(row.path);
            }}
          >
            <RowIcon row={row} />
            <span className="codect-row-label">{row.label}</span>
            {status !== undefined && (
              <span className={`codect-badge codect-badge-${status}`}>
                {BADGE_LETTER[status]}
              </span>
            )}
          </div>
        );
      })}
    </div>
  );
}
