/**
 * declaration-diff.ts — declaration-level outline comparison.
 *
 * Pure and React-free: given a `DiffFile`, match the two sides' outlines on
 * `stable_key` and report the declarations that added, deleted, or changed.
 *
 * A container's outline `signature` is the Signatures-superset fragment that
 * includes its members, so a change nested inside it also changes the
 * container's signature. Left alone that reports one nested edit twice. We
 * therefore suppress any reported change whose ancestor is also reported: a
 * container is "explained" by its descendants, and only the leaf declaration
 * is surfaced. A container that changed with no reported descendant is still
 * reported. Declarations the active mode drops (`retained_in_mode: false`) are
 * ignored on both sides.
 */

import type { DiffChange, DiffFile, OutlineItem } from "../shared/schema";

/** Whether the active mode's projection retains this declaration. */
function retained(item: OutlineItem): boolean {
  return item.retained_in_mode !== false;
}

/** Compare the two sides of a file's outline into declaration-level changes. */
export function diffChanges(file: DiffFile): DiffChange[] {
  const baseItems = new Map<string, OutlineItem>(
    (file.base?.outline ?? []).map((item) => [item.stable_key, item])
  );
  const targetItems = new Map<string, OutlineItem>(
    (file.target?.outline ?? []).map((item) => [item.stable_key, item])
  );

  // A key's parent may only exist on one side, so build the ancestry from both.
  const parentOf = new Map<string, string | null>();
  for (const item of file.base?.outline ?? []) {
    parentOf.set(item.stable_key, item.parent_key);
  }
  for (const item of file.target?.outline ?? []) {
    parentOf.set(item.stable_key, item.parent_key);
  }

  const changes: DiffChange[] = [];
  for (const [key, item] of baseItems) {
    if (!targetItems.has(key) && retained(item)) {
      changes.push({ status: "deleted", item, base_text: item.signature, target_text: null });
    }
  }
  for (const [key, item] of targetItems) {
    if (!baseItems.has(key) && retained(item)) {
      changes.push({ status: "added", item, base_text: null, target_text: item.signature });
    }
  }
  for (const [key, item] of targetItems) {
    const before = baseItems.get(key);
    if (
      before !== undefined &&
      retained(before) &&
      retained(item) &&
      before.signature !== item.signature
    ) {
      changes.push({
        status: "changed",
        item,
        base_text: before.signature,
        target_text: item.signature,
      });
    }
  }

  // Suppress every reported change that has another reported change somewhere
  // in its descendant chain, so a container is not repeated alongside the leaf
  // that actually changed.
  const reported = new Set(changes.map((change) => change.item.stable_key));
  const suppressed = new Set<string>();
  for (const change of changes) {
    let parent = parentOf.get(change.item.stable_key) ?? null;
    while (parent !== null) {
      if (reported.has(parent)) suppressed.add(parent);
      parent = parentOf.get(parent) ?? null;
    }
  }

  return changes.filter((change) => !suppressed.has(change.item.stable_key));
}
