/**
 * outline.ts — Pure declaration-outline helpers for the codect DSH views.
 *
 * The wire outline (`file.outline`) is a flat, complete declaration list with
 * `parent_key` links. The views render it indented by nesting depth and keyed
 * by `(path, stable_key)`. No React and no DOM.
 */

import type { OutlineItem } from "../shared/schema";

export interface OutlineRow {
  depth: number;
  item: OutlineItem;
}

type Keyed = Pick<OutlineItem, "stable_key" | "parent_key">;

function depthOf(item: Keyed, byKey: ReadonlyMap<string, Keyed>): number {
  let depth = 0;
  let parent = item.parent_key;
  const seen = new Set<string>([item.stable_key]);
  while (parent !== null && parent !== undefined && !seen.has(parent)) {
    seen.add(parent);
    const next = byKey.get(parent);
    if (next === undefined) {
      return depth + 1;
    }
    depth += 1;
    parent = next.parent_key;
  }
  return depth;
}

export function outlineRows(items: readonly OutlineItem[]): OutlineRow[] {
  const byKey = new Map<string, Keyed>(items.map((item) => [item.stable_key, item]));
  return items.map((item) => ({ depth: depthOf(item, byKey), item }));
}
