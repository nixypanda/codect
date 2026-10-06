/**
 * Unit tests for the plugin's pure helpers.
 *
 * Node's built-in TypeScript type stripping lets the test import the `.ts`
 * sources directly, so the tree and address logic is covered without a build.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { buildTreeRows } from "../src/client-codect/tree.ts";
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
