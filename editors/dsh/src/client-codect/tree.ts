/**
 * tree.ts — Pure file-tree model for the codect DSH sidebar views.
 *
 * Each directory row is emitted once at the position of its first descendant,
 * and a collapsed directory skips its whole subtree. Within a level, entries
 * are ordered like the DSH Workspace files plugin: directories first, then a
 * natural case-insensitive name order (`Intl.Collator`, numeric), with a raw
 * name tie-break so the order is total and deterministic.
 *
 * No React and no DOM: this module is unit-testable on its own.
 */

export type TreeKind = "dir" | "file";

/** One visible row of the file tree. */
export interface TreeRow {
  /** Indentation depth, 0 for a top-level entry. */
  depth: number;
  /** The final path component (the row label). */
  label: string;
  /** The repo-relative path (directory path for directories). */
  path: string;
  kind: TreeKind;
  /** Directories only: whether the directory is expanded. */
  expanded: boolean;
}

interface Node {
  name: string;
  path: string;
  kind: TreeKind;
  children: Map<string, Node>;
}

function makeNode(name: string, path: string, kind: TreeKind): Node {
  return { name, path, kind, children: new Map() };
}

/**
 * Flatten repo-relative file paths into visible tree rows.
 *
 * @param paths - file paths, e.g. `show.files[].path`.
 * @param collapsed - directory paths whose subtrees are folded.
 */
export function buildTreeRows(
  paths: readonly string[],
  collapsed: ReadonlySet<string>
): TreeRow[] {
  const root = makeNode("", "", "dir");

  for (const raw of paths) {
    const parts = raw.split("/").filter((part) => part.length > 0);
    if (parts.length === 0) continue;
    let node = root;
    for (let index = 0; index < parts.length; index += 1) {
      const isFile = index === parts.length - 1;
      const path = parts.slice(0, index + 1).join("/");
      let child = node.children.get(parts[index]);
      if (child === undefined) {
        child = makeNode(parts[index], path, isFile ? "file" : "dir");
        node.children.set(parts[index], child);
      }
      node = child;
    }
  }

  const rows: TreeRow[] = [];
  const walk = (node: Node, depth: number): void => {
    const children = [...node.children.values()].sort(compareNodes);
    for (const child of children) {
      if (child.kind === "dir") {
        const expanded = !collapsed.has(child.path);
        rows.push({
          depth,
          label: child.name,
          path: child.path,
          kind: "dir",
          expanded,
        });
        if (expanded) walk(child, depth + 1);
      } else {
        rows.push({
          depth,
          label: child.name,
          path: child.path,
          kind: "file",
          expanded: false,
        });
      }
    }
  };
  walk(root, 0);
  return rows;
}

/** Natural, case-insensitive order for sibling names (the Workspace files look). */
const nameCollator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

function compareNodes(a: Node, b: Node): number {
  if (a.kind !== b.kind) return a.kind === "dir" ? -1 : 1;
  const byName = nameCollator.compare(a.name, b.name);
  if (byName !== 0) return byName;
  if (a.name < b.name) return -1;
  if (a.name > b.name) return 1;
  return 0;
}
