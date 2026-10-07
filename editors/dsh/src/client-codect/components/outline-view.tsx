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
}

/** First non-empty line of a possibly multi-line canonical fragment. */
export function firstLine(text: string): string {
  const line = text.split("\n", 1)[0] ?? "";
  return line.trim();
}

export function OutlineView({ rows, onJump }: OutlineViewProps) {
  return (
    <div className="codect-outline" role="tree">
      {rows.map(({ depth, item }) => {
        const retained = item.retained_in_mode !== false;
        return (
          <div
            key={item.stable_key}
            className={`codect-outline-row${retained ? "" : " codect-outline-dim"}`}
            style={{ paddingLeft: 6 + depth * 12 }}
            role="treeitem"
            title={item.signature}
            onClick={() => onJump(item.span.start_line)}
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
