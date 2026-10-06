/**
 * Unit tests for the plugin's pure helpers.
 *
 * Node's built-in TypeScript type stripping lets the test import the `.ts`
 * sources directly, so the tree and address logic is covered without a build.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { buildTreeRows } from "../src/client-codect/tree.ts";
import { diffChanges } from "../src/client-codect/declaration-diff.ts";
import {
  fileAddressFor,
  sessionFileAddress,
} from "../src/client-codect/address.ts";

test("buildTreeRows splits paths into directory and file rows", () => {
  const rows = buildTreeRows(
    ["src/a.rs", "src/b.rs", "README.md"],
    new Set()
  );
  assert.deepEqual(
    rows.map((row) => [row.depth, row.kind, row.path, row.expanded]),
    [
      [0, "dir", "src", true],
      [1, "file", "src/a.rs", false],
      [1, "file", "src/b.rs", false],
      [0, "file", "README.md", false],
    ]
  );
});

test("buildTreeRows hides the subtree of a collapsed directory", () => {
  const rows = buildTreeRows(
    ["src/a.rs", "src/b.rs", "README.md"],
    new Set(["src"])
  );
  assert.deepEqual(
    rows.map((row) => [row.depth, row.kind, row.path, row.expanded]),
    [
      [0, "dir", "src", false],
      [0, "file", "README.md", false],
    ]
  );
});

test("buildTreeRows orders directories before files at each level", () => {
  // Directory-first, natural name order — matches the Workspace files plugin,
  // so `src/lib/` precedes its sibling file `src/lib.rs`.
  const rows = buildTreeRows(["src/lib.rs", "src/lib/a.rs"], new Set());
  assert.deepEqual(
    rows.map((row) => [row.depth, row.kind, row.path]),
    [
      [0, "dir", "src"],
      [1, "dir", "src/lib"],
      [2, "file", "src/lib/a.rs"],
      [1, "file", "src/lib.rs"],
    ]
  );
});

test("buildTreeRows places directories before files regardless of name", () => {
  const rows = buildTreeRows(["a.rs", "z/x.rs"], new Set());
  assert.deepEqual(
    rows.map((row) => row.path),
    ["z", "z/x.rs", "a.rs"]
  );
});

test("buildTreeRows orders names naturally and case-insensitively", () => {
  const rows = buildTreeRows(["a10.rs", "a2.rs", "B.rs", "c.rs"], new Set());
  assert.deepEqual(
    rows.map((row) => row.label),
    ["a2.rs", "a10.rs", "B.rs", "c.rs"]
  );
});

test("buildTreeRows ignores empty and leading-slash components", () => {
  const rows = buildTreeRows(["/a/b.rs", ""], new Set());
  assert.deepEqual(
    rows.map((row) => row.path),
    ["a", "a/b.rs"]
  );
});

test("sessionFileAddress encodes segments but keeps colons", () => {
  assert.equal(
    sessionFileAddress("s1", "src/a b.rs"),
    "dsh-resource://file/session/s1/src/a%20b.rs"
  );
  assert.equal(
    sessionFileAddress("C:", "a.rs"),
    "dsh-resource://file/session/C:/a.rs"
  );
});

test("fileAddressFor maps relative and workspace-absolute paths", () => {
  assert.equal(
    fileAddressFor("s1", "/home/u/proj", "src/a.rs"),
    "dsh-resource://file/session/s1/src/a.rs"
  );
  assert.equal(
    fileAddressFor("s1", "/home/u/proj", "/home/u/proj/src/a.rs"),
    "dsh-resource://file/session/s1/src/a.rs"
  );
  assert.equal(
    fileAddressFor("s1", "/home/u/proj", "/elsewhere/a.rs"),
    "dsh-resource://file/session/s1//elsewhere/a.rs"
  );
});

// ---------------------------------------------------------------------------
// diffChanges — synthetic DiffFile objects, no CLI or git
// ---------------------------------------------------------------------------

/** Minimal outline item; `diffChanges` only reads these fields. */
function item(stable_key, parent_key, name, signature, kind = "field", retained_in_mode) {
  const outlineItem = {
    stable_key,
    parent_key,
    kind,
    name,
    span: { start_line: 1, end_line: 1, start_byte: 0, end_byte: 0 },
    signature,
  };
  if (retained_in_mode !== undefined) outlineItem.retained_in_mode = retained_in_mode;
  return outlineItem;
}

/** A diff side with an outline; projection is irrelevant to `diffChanges`. */
function side(outline) {
  return { snapshot_id: "test", projection: { text: "", items: [] }, outline };
}

/** A modified `DiffFile` from two outlines. */
function diffFile(baseOutline, targetOutline) {
  return {
    path: "src/a.rs",
    language: "rust",
    status: "modified",
    base: side(baseOutline),
    target: side(targetOutline),
    equal: false,
  };
}

test("diffChanges reports a nested change once, not its container", () => {
  const file = diffFile(
    [
      item("Outer", null, "Outer", "struct Outer", "struct"),
      item("Outer::inner", "Outer", "inner", "inner: Inner"),
      item("Inner", null, "Inner", "struct Inner { x: i32 }", "struct"),
      item("Inner::x", "Inner", "x", "x: i32"),
    ],
    [
      item("Outer", null, "Outer", "struct Outer", "struct"),
      item("Outer::inner", "Outer", "inner", "inner: Inner"),
      item("Inner", null, "Inner", "struct Inner { x: i64 }", "struct"),
      item("Inner::x", "Inner", "x", "x: i64"),
    ]
  );

  const changes = diffChanges(file);
  assert.equal(changes.length, 1);
  assert.equal(changes[0].status, "changed");
  assert.equal(changes[0].item.stable_key, "Inner::x");
  assert.equal(changes[0].base_text, "x: i32");
  assert.equal(changes[0].target_text, "x: i64");
});

test("diffChanges reports a rename as deleted + added without a container change", () => {
  const file = diffFile(
    [
      item("S", null, "S", "struct S { x: i32 }", "struct"),
      item("S::x", "S", "x", "x: i32"),
    ],
    [
      item("S", null, "S", "struct S { y: i32 }", "struct"),
      item("S::y", "S", "y", "y: i32"),
    ]
  );

  const changes = diffChanges(file);
  const deleted = changes.filter((change) => change.status === "deleted");
  const added = changes.filter((change) => change.status === "added");
  assert.deepEqual(deleted.map((change) => change.item.stable_key), ["S::x"]);
  assert.deepEqual(added.map((change) => change.item.stable_key), ["S::y"]);
  // The container differs only because its members did; it is explained away.
  assert.equal(changes.some((change) => change.item.stable_key === "S"), false);
  assert.equal(changes.some((change) => change.status === "changed"), false);
});

test("diffChanges ignores declarations not retained in the active mode", () => {
  const file = diffFile(
    [item("x", null, "x", "x: i32")],
    [item("x", null, "x", "x: i64", "field", false)]
  );

  assert.deepEqual(diffChanges(file), []);
});

test("diffChanges reports a container change with no descendant change", () => {
  const file = diffFile(
    [
      item("Inner", null, "Inner", "struct Inner<T>", "struct"),
      item("Inner::x", "Inner", "x", "x: i32"),
    ],
    [
      item("Inner", null, "Inner", "struct Inner<U>", "struct"),
      item("Inner::x", "Inner", "x", "x: i32"),
    ]
  );

  const changes = diffChanges(file);
  assert.equal(changes.length, 1);
  assert.equal(changes[0].status, "changed");
  assert.equal(changes[0].item.stable_key, "Inner");
});
