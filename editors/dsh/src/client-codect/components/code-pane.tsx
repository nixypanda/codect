/**
 * code-pane.tsx — The canonical projection text of one file.
 *
 * Prefers the shared `ReadBlock` card (line gutter, lazy syntax highlighting,
 * wrap and copy controls, `--dsw-*` theme). Falls back to a plain `<pre>` when
 * the primitives are unavailable.
 */

import React, { useMemo } from "react";
import { ReadBlock } from "../primitives";
import type { CodeLabels } from "../labels";

export interface CodePaneProps {
  text: string;
  language: string;
  title?: string;
  labels: CodeLabels;
}

/** Content lines, matching the output cards' terminator rule (no phantom last line). */
export function textToLines(text: string): { number: number; text: string }[] {
  if (text === "") return [];
  const body = text.endsWith("\n") ? text.slice(0, -1) : text;
  return body.split("\n").map((line, index) => ({ number: index + 1, text: line }));
}

export function CodePane({ text, language, title, labels }: CodePaneProps) {
  const lines = useMemo(() => textToLines(text), [text]);
  if (typeof ReadBlock === "function") {
    return (
      <ReadBlock
        label={title}
        labels={labels}
        lines={lines}
        totalLines={lines.length}
        lang={language}
        maxLines={Math.max(lines.length, 1)}
      />
    );
  }
  return (
    <pre className="codect-fallback-code">
      {lines.map((line) => `${line.number} ${line.text}`).join("\n")}
    </pre>
  );
}
