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
 *
 * A moved file's stable keys are path-prefixed on both sides. Rebase the base
 * keys onto the target path before matching so an exact move reports no
 * declaration changes rather than deleting and re-adding every declaration.
 */

import type { DiffChange, DiffFile, OutlineItem } from "../shared/schema";

function retained(item: OutlineItem): boolean {
  return item.retained_in_mode !== false;
}

function rebaseKey(file: DiffFile, key: string | null): string | null {
  if (key === null || file.status !== "moved") return key;
  const prefix = `${file.old_path}::`;
  return key.startsWith(prefix)
    ? `${file.path}${key.slice(file.old_path.length)}`
    : key;
}

function rebaseItem(file: DiffFile, item: OutlineItem): OutlineItem {
  if (file.status !== "moved") return item;
  const stableKey = rebaseKey(file, item.stable_key);
  const parentKey = rebaseKey(file, item.parent_key);
  if (stableKey === item.stable_key && parentKey === item.parent_key) return item;
  return {
    ...item,
    stable_key: stableKey ?? item.stable_key,
    parent_key: parentKey,
  };
}

export function diffChanges(file: DiffFile): DiffChange[] {
  const baseOutline = (file.base?.outline ?? []).map((item) =>
    rebaseItem(file, item)
  );
  const targetOutline = (file.target?.outline ?? []).map((item) =>
    rebaseItem(file, item)
  );
  const baseItems = new Map<string, OutlineItem>(
    baseOutline.map((item) => [item.stable_key, item])
  );
  const targetItems = new Map<string, OutlineItem>(
    targetOutline.map((item) => [item.stable_key, item])
  );

  const parentOf = new Map<string, string | null>();
  for (const item of baseOutline) {
    parentOf.set(item.stable_key, item.parent_key);
  }
  for (const item of targetOutline) {
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
