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
  basePath,
  canNavigateChange,
  canNavigatePath,
  decodeDisplayPath,
  DIFF_PATH_ENCODING,
  moveLabel,
  movedFrom,
  navigationPath,
  pathForChange,
  showNavigationEncoding,
  targetPath,
} from "../src/client-codect/move.ts";
import {
  fileAddressFor,
  repoFileAddress,
  sessionFileAddress,
} from "../src/client-codect/address.ts";
import { classifyExecError } from "../src/api-codect/exec-error.ts";
import { TYPERT } from "../src/api-codect/typert.host.ts";

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

test("repoFileAddress preserves literal backslashes and controls in decoded paths", () => {
  assert.equal(
    repoFileAddress("s1", "src/plain.rs"),
    "dsh-resource://file/session/s1/src/plain.rs"
  );
  assert.equal(
    repoFileAddress("s1", String.raw`src/\path.rs`),
    "dsh-resource://file/session/s1/src/%5Cpath.rs"
  );
  assert.equal(
    repoFileAddress("s1", "src/a\nb.rs"),
    "dsh-resource://file/session/s1/src/a%0Ab.rs"
  );
  assert.equal(
    repoFileAddress("s1", "src/☃.rs"),
    "dsh-resource://file/session/s1/src/%E2%98%83.rs"
  );
});

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

function side(outline, text = "") {
  return { snapshot_id: "test", projection: { text, items: [] }, outline };
}

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

function movedFile({
  oldPath = "src/old.rs",
  path = "src/new.rs",
  equal = true,
  before = "",
  after = "",
  baseOutline = [],
  targetOutline = [],
} = {}) {
  return {
    path,
    old_path: oldPath,
    language: "rust",
    status: "moved",
    base: side(baseOutline, before),
    target: side(targetOutline, after),
    equal,
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

test("decodeDisplayPath distinguishes literal escapes from encoded controls and bytes", () => {
  assert.equal(decodeDisplayPath("src/plain.rs"), "src/plain.rs");
  assert.equal(
    decodeDisplayPath(String.raw`src/\\path.rs`),
    String.raw`src/\path.rs`
  );
  assert.equal(
    decodeDisplayPath(String.raw`src/\\xFF.rs`),
    String.raw`src/\xFF.rs`
  );
  assert.equal(decodeDisplayPath(String.raw`src/\xFF.rs`), null);
  assert.equal(
    decodeDisplayPath(String.raw`src/\\n.rs`),
    String.raw`src/\n.rs`
  );
  assert.equal(decodeDisplayPath(String.raw`src/\n.rs`), "src/\n.rs");
  assert.equal(
    decodeDisplayPath(String.raw`src/\\t.rs`),
    String.raw`src/\t.rs`
  );
  assert.equal(decodeDisplayPath(String.raw`src/\t.rs`), "src/\t.rs");
  assert.equal(decodeDisplayPath(String.raw`src/\r.rs`), "src/\r.rs");
  assert.equal(decodeDisplayPath(String.raw`src/\x01.rs`), "src/\x01.rs");
  assert.equal(decodeDisplayPath(String.raw`src/\x7F.rs`), "src/\x7F.rs");
  assert.equal(decodeDisplayPath("src/☃.rs"), "src/☃.rs");
  assert.equal(decodeDisplayPath("src/😀.rs"), "src/😀.rs");
});

test("decodeDisplayPath rejects malformed display escapes", () => {
  assert.equal(decodeDisplayPath("src/trailing\\"), null);
  assert.equal(decodeDisplayPath(String.raw`src\q.rs`), null);
  assert.equal(decodeDisplayPath(String.raw`src\x1.rs`), null);
  assert.equal(decodeDisplayPath(String.raw`src\xGG.rs`), null);
  assert.equal(decodeDisplayPath(String.raw`src\x80.rs`), null);
});

test("legacy show paths never decode a literal backslash escape", () => {
  const literal = String.raw`src/lit\x41.rs`;
  assert.equal(navigationPath(literal, "legacy"), literal);
  assert.notEqual(navigationPath(literal, "legacy"), "src/litA.rs");
  assert.equal(canNavigatePath(literal, "legacy"), true);

  const marked = String.raw`src/lit\\x41.rs`;
  assert.equal(navigationPath(marked, DIFF_PATH_ENCODING), literal);
  assert.equal(canNavigatePath(marked, DIFF_PATH_ENCODING), true);

  assert.equal(navigationPath(String.raw`src/back\slash.rs`, "legacy"), String.raw`src/back\slash.rs`);
  assert.equal(navigationPath(String.raw`src/\xFF.rs`, "legacy"), null);
  assert.equal(canNavigatePath(String.raw`src/\xFF.rs`, "legacy"), false);
  assert.equal(navigationPath("src/a.rs", "unsupported"), null);
});

test("old and new show documents share v1 but navigate their paths differently", () => {
  const literal = String.raw`src/lit\x41.rs`;
  const legacy = showDocument({ files: [showFile(literal)] });
  const current = showDocument({
    path_encoding: "codect.path.v1",
    files: [showFile(String.raw`src/lit\\x41.rs`)],
  });

  assert.equal(ShowResult.safeParse(legacy).success, true);
  assert.equal(ShowResult.safeParse(current).success, true);

  const legacyEncoding = showNavigationEncoding(legacy);
  const currentEncoding = showNavigationEncoding(current);
  assert.equal(legacyEncoding, "legacy");
  assert.equal(currentEncoding, "codect.path.v1");
  assert.equal(navigationPath(legacy.files[0].path, legacyEncoding), literal);
  assert.notEqual(navigationPath(legacy.files[0].path, legacyEncoding), "src/litA.rs");
  assert.equal(navigationPath(current.files[0].path, currentEncoding), literal);
});

test("showNavigationEncoding honors the optional marker and rejects unknown values", () => {
  assert.equal(showNavigationEncoding(null), "legacy");
  assert.equal(showNavigationEncoding({}), "legacy");
  assert.equal(
    showNavigationEncoding({ path_encoding: "codect.path.v1" }),
    "codect.path.v1"
  );
  assert.equal(
    showNavigationEncoding({ path_encoding: "codect.path.v2" }),
    "unsupported"
  );
  assert.equal(
    ShowResult.safeParse(showDocument({ path_encoding: "codect.path.v1" }))
      .success,
    true
  );
  assert.equal(
    ShowResult.safeParse(showDocument({ path_encoding: "codect.path.v2" }))
      .success,
    false
  );
  assert.equal(ShowResult.safeParse(showDocument()).success, true);
});

test("move helpers keep the source and target paths distinct", () => {
  const move = movedFile({ before: "pub struct S;\n", after: "pub struct S;\n" });
  assert.equal(movedFrom(move), "src/old.rs");
  assert.equal(basePath(move), "src/old.rs");
  assert.equal(targetPath(move), "src/new.rs");
  assert.equal(moveLabel(move), "src/old.rs → src/new.rs");
  assert.equal(pathForChange(move, "deleted"), "src/old.rs");
  assert.equal(pathForChange(move, "changed"), "src/new.rs");
  assert.equal(canNavigateChange(move, "deleted"), false);
  assert.equal(canNavigateChange(move, "changed"), true);
  assert.equal(canNavigateChange(move, "added"), true);

  const deleted = {
    ...diffFile([], []),
    status: "deleted",
    base: side([]),
    target: null,
  };
  assert.equal(canNavigateChange(deleted, "deleted"), true);

  const modified = diffFile([], []);
  assert.equal(movedFrom(modified), undefined);
  assert.equal(moveLabel(modified), undefined);
  assert.equal(basePath(modified), "src/a.rs");
  assert.equal(targetPath(modified), "src/a.rs");
  assert.equal(canNavigateChange(modified, "deleted"), true);
});

test("diffChanges compares the two projections of a move", () => {
  const move = movedFile({
    baseOutline: [item("S::x", "S", "x", "x: i32")],
    targetOutline: [item("S::x", "S", "x", "x: i32")],
  });
  assert.deepEqual(diffChanges(move), []);
});

test("diffChanges rebases path-prefixed stable keys for a move", () => {
  const move = movedFile({
    oldPath: "src/old.rs",
    path: "src/new.rs",
    baseOutline: [
      item("src/old.rs::type::S", null, "S", "struct S", "struct"),
      item("src/old.rs::type::S::field::x", "src/old.rs::type::S", "x", "x: i32"),
    ],
    targetOutline: [
      item("src/new.rs::type::S", null, "S", "struct S", "struct"),
      item("src/new.rs::type::S::field::x", "src/new.rs::type::S", "x", "x: i32"),
    ],
  });
  assert.deepEqual(diffChanges(move), []);
});

const ShowResult = TYPERT.invocations.find(
  (invocation) => invocation.id === "@nixypanda/dsh-codect#codect/show"
).result.create();

function showDocument(extra = {}) {
  return {
    schema: "codect.show.v1",
    input: "revision",
    revision: "HEAD",
    mode: "types",
    files: [],
    ...extra,
  };
}

function showFile(path) {
  return {
    path,
    language: "rust",
    projection: { text: "", items: [] },
    outline: [],
  };
}

const DiffResult = TYPERT.invocations.find(
  (invocation) => invocation.id === "@nixypanda/dsh-codect#codect/diff"
).result.create();

function diffDocument(files) {
  return {
    schema: "codect.diff.v2",
    mode: "types",
    base: { kind: "commit", revision: "HEAD~1", id: "base" },
    target: { kind: "commit", revision: "HEAD", id: "target" },
    files,
  };
}

test("DiffResult accepts a moved file with projection-derived equality", () => {
  const exact = movedFile({ before: "pub struct S;\n", after: "pub struct S;\n" });
  const edited = movedFile({
    oldPath: "src/old2.rs",
    path: "src/new2.rs",
    equal: false,
    before: "pub struct S { x: i32 }\n",
    after: "pub struct S { x: i64 }\n",
  });
  assert.equal(DiffResult.safeParse(diffDocument([exact, edited])).success, true);
});

test("DiffResult rejects a move whose paths are not distinct", () => {
  const file = movedFile({ oldPath: "src/new.rs", path: "src/new.rs" });
  assert.equal(DiffResult.safeParse(diffDocument([file])).success, false);
});

test("DiffResult rejects moved files without both sides or a matching equal flag", () => {
  const missingSide = { ...movedFile(), target: null };
  assert.equal(DiffResult.safeParse(diffDocument([missingSide])).success, false);

  const wrongEqual = movedFile({ equal: true, before: "a", after: "b" });
  assert.equal(DiffResult.safeParse(diffDocument([wrongEqual])).success, false);
});

test("DiffResult keeps old_path and equal: true out of other statuses", () => {
  const added = {
    path: "src/new.rs",
    language: "rust",
    status: "added",
    base: null,
    target: side([]),
    equal: false,
  };
  const deleted = {
    path: "src/old.rs",
    language: "rust",
    status: "deleted",
    base: side([]),
    target: null,
    equal: false,
  };
  const modified = {
    path: "src/a.rs",
    language: "rust",
    status: "modified",
    base: side([]),
    target: side([]),
    equal: false,
  };
  assert.equal(
    DiffResult.safeParse(diffDocument([added, deleted, modified])).success,
    true
  );
  for (const file of [added, deleted, modified]) {
    assert.equal(
      DiffResult.safeParse(
        diffDocument([{ ...file, old_path: "src/old.rs" }])
      ).success,
      false
    );
    assert.equal(
      DiffResult.safeParse(diffDocument([{ ...file, equal: true }])).success,
      false
    );
  }
});

test("classifyExecError reports ENOENT as a missing binary", () => {
  assert.equal(classifyExecError({ code: "ENOENT" }), "binary-missing");
});

test("classifyExecError reports a killed child as a timeout", () => {
  assert.equal(
    classifyExecError({ killed: true, signal: "SIGTERM" }),
    "timeout"
  );
});

test("classifyExecError reports a signal without killed as a timeout", () => {
  assert.equal(classifyExecError({ signal: "SIGTERM" }), "timeout");
});

test("classifyExecError reports a plain failure", () => {
  assert.equal(classifyExecError({}), "failed");
});

test("classifyExecError reports a numeric exit code as a plain failure", () => {
  assert.equal(classifyExecError({ code: 1 }), "failed");
});
