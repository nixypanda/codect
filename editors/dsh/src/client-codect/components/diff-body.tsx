/**
 * diff-body.tsx — The projection diff of one changed file.
 *
 * Both layouts (unified and side-by-side) render through the optional `diffView`
 * client service provided by the community `dsh-diff-view` plugin, which applies
 * syntax highlighting (`useCodeHighlighter`), intra-line word marks, real line
 * numbers, and context collapse. That service is not part of the platform module
 * table, so when it is absent the body degrades to the platform `DiffBlock`
 * (unified, plain text) and finally to a plain text dump.
 */

import React, { useMemo } from "react";
import { DiffBlock } from "../primitives";
import type { CodeLabels } from "../labels";
import type { DiffFile } from "../../shared/schema";

export type DiffLayout = "unified" | "split";

export interface DiffBodyProps {
  file: DiffFile;
  labels: CodeLabels;
  /** `split` and `unified` both need `diffView` for highlighting. */
  layout?: DiffLayout;
  /** Optional `dsh-diff-view` client service (see `ctx.diffView`). */
  diffView?: any;
}

/** The canonical projection texts for a file's two sides ("" when absent). */
export function diffTexts(file: DiffFile): { before: string; after: string } {
  return {
    before: file.base?.projection.text ?? "",
    after: file.target?.projection.text ?? "",
  };
}

function fallbackDiff(file: DiffFile): string {
  const { before, after } = diffTexts(file);
  return [
    `--- ${file.path} (base)`,
    before,
    `+++ ${file.path} (target)`,
    after,
  ].join("\n");
}

export function DiffBody({ file, labels, layout = "unified", diffView }: DiffBodyProps) {
  const { before, after } = diffTexts(file);

  // Build the highlighted diff component once per file/layout. `dsh-diff-view`
  // computes its hunks eagerly here; a malformed service is treated as absent.
  const component = useMemo(() => {
    if (diffView === null || diffView === undefined) return null;
    if (typeof diffView.diffFileComponent !== "function") return null;
    try {
      return diffView.diffFileComponent({
        path: file.path,
        before,
        after,
        mode: layout,
        showToggle: false,
      });
    } catch {
      return null;
    }
  }, [diffView, layout, file.path, before, after]);

  if (component !== null && component !== undefined) {
    return (
      <div className="codect-diff-surface">{React.createElement(component)}</div>
    );
  }

  // No `dsh-diff-view`: keep the platform unified renderer (no highlighting).
  if (typeof DiffBlock === "function") {
    const diffs = [
      {
        path: file.path,
        oldText: before,
        newText: after,
      },
    ];
    return (
      <DiffBlock diffs={diffs} labels={labels} maxLines={Number.MAX_SAFE_INTEGER} />
    );
  }

  return <pre className="codect-fallback-code">{fallbackDiff(file)}</pre>;
}
