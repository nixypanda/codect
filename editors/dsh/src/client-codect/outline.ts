/**
 * outline.ts — Pure declaration-outline helpers for the codect DSH views.
 *
 * The wire outline (`file.outline`) is a flat, complete declaration list with
 * `parent_key` links. The views render it indented by nesting depth and keyed
 * by `(path, stable_key)`. No React and no DOM.
 */

import type { OutlineItem } from "../shared/schema";

/** One outline entry with its computed nesting depth. */
export interface OutlineRow {
  depth: number;
  item: OutlineItem;
}

/** The minimal outline shape the depth walk needs. */
type Keyed = Pick<OutlineItem, "stable_key" | "parent_key">;

function depthOf(item: Keyed, byKey: ReadonlyMap<string, Keyed>): number {
  let depth = 0;
  let parent = item.parent_key;
  const seen = new Set<string>([item.stable_key]);
  while (parent !== null && parent !== undefined && !seen.has(parent)) {
    seen.add(parent);
    const next = byKey.get(parent);
    if (next === undefined) {
      // A dangling parent link still contributes one level.
      return depth + 1;
    }
    depth += 1;
    parent = next.parent_key;
  }
  return depth;
}

/** Decorate an outline with nesting depth, preserving declaration order. */
export function outlineRows(items: readonly OutlineItem[]): OutlineRow[] {
  const byKey = new Map<string, Keyed>(items.map((item) => [item.stable_key, item]));
  return items.map((item) => ({ depth: depthOf(item, byKey), item }));
}
