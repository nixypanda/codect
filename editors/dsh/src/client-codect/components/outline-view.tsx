/**
 * outline-view.tsx — The nested declaration outline of one file.
 *
 * Entries are indented by `parent_key` depth and show the declaration kind,
 * name, and signature. A `signature` kind that the active mode does not retain
 * is dimmed (`retained_in_mode === false`).
 */

import React from "react";
import type { OutlineRow } from "../outline";

export interface OutlineViewProps {
  rows: OutlineRow[];
  onJump: (line: number) => void;
  /** False when the selected file has no safely navigable host path. */
  canJump?: boolean;
}

export function firstLine(text: string): string {
  const line = text.split("\n", 1)[0] ?? "";
  return line.trim();
}

export function OutlineView({ rows, onJump, canJump = true }: OutlineViewProps) {
  return (
    <div className="codect-outline" role="tree">
      {rows.map(({ depth, item }) => {
        const retained = item.retained_in_mode !== false;
        return (
          <div
            key={item.stable_key}
            className={`codect-outline-row${retained ? "" : " codect-outline-dim"}${
              canJump ? "" : " codect-outline-row-static"
            }`}
            style={{ paddingLeft: 6 + depth * 12 }}
            role="treeitem"
            title={
              canJump
                ? item.signature
                : `${item.signature} (source path is not navigable)`
            }
            aria-disabled={canJump ? undefined : true}
            onClick={canJump ? () => onJump(item.span.start_line) : undefined}
          >
            <span className="codect-outline-kind">{item.kind}</span>
            <span className="codect-outline-name">{item.name}</span>
            <span className="codect-outline-sig">{firstLine(item.signature)}</span>
          </div>
        );
      })}
    </div>
  );
}
