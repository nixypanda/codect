/**
 * Independent acceptance tests for file moves in codect.diff.v2 as the DSH
 * plugin consumes them.
 *
 * This file deliberately builds its own throwaway repository and exercises the
 * real CLI instead of reusing the implementer's fixtures. It covers exact,
 * edited, and body-only moves, scope selection by either path, target-keyed
 * rows, runtime schema acceptance/rejection, stable-key rebasing in every
 * supported language, and the non-navigable deleted declaration inside a move.
 *
 * There is no React renderer harness in this package; component wiring is
 * covered by the pure helpers plus source inspection, not by mounting JSX.
 */

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { diffChanges } from "../src/client-codect/declaration-diff.ts";
import { buildTreeRows } from "../src/client-codect/tree.ts";
import {
  basePath,
  canNavigateChange,
  canNavigatePath,
  DIFF_PATH_ENCODING,
  moveLabel,
  movedFrom,
  navigationPath,
  pathForChange,
  showNavigationEncoding,
  targetPath,
} from "../src/client-codect/move.ts";
import { repoFileAddress } from "../src/client-codect/address.ts";
import { TYPERT } from "../src/api-codect/typert.host.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

// Explicit path-display fixtures. RAW_* contain the actual filesystem
// characters; DISPLAY_* are the strings codect emits after JSON decoding.
// These are hard-coded from docs/agents/CONTRACT.md and the contract, not
// produced by a copy of the encoder.
const ENCODED_PATHS = [
  {
    raw: "src/back\\slash.rs",
    display: String.raw`src/back\\slash.rs`,
    address: "dsh-resource://file/session/s1/src/back%5Cslash.rs",
  },
  {
    raw: "src/new\nline.rs",
    display: String.raw`src/new\nline.rs`,
    address: "dsh-resource://file/session/s1/src/new%0Aline.rs",
  },
  {
    raw: "src/tab\tname.rs",
    display: String.raw`src/tab\tname.rs`,
    address: "dsh-resource://file/session/s1/src/tab%09name.rs",
  },
  {
    raw: "src/ctrl\x01name.rs",
    display: String.raw`src/ctrl\x01name.rs`,
    address: "dsh-resource://file/session/s1/src/ctrl%01name.rs",
  },
  {
    raw: "src/☃ snow.rs",
    display: "src/☃ snow.rs",
    address: "dsh-resource://file/session/s1/src/%E2%98%83%20snow.rs",
  },
];

const LITERAL_RAW_PATH = String.raw`src/literal\xFF.rs`;
const LITERAL_DISPLAY_PATH = String.raw`src/literal\\xFF.rs`;
const LITERAL_ADDRESS = "dsh-resource://file/session/s1/src/literal%5CxFF.rs";

const MOVE_OLD_RAW_PATH = "src/move\nold.rs";
const MOVE_NEW_RAW_PATH = "src/move\tnew.rs";
const MOVE_OLD_DISPLAY_PATH = String.raw`src/move\nold.rs`;
const MOVE_NEW_DISPLAY_PATH = String.raw`src/move\tnew.rs`;
const MOVE_OLD_ADDRESS = "dsh-resource://file/session/s1/src/move%0Aold.rs";
const MOVE_NEW_ADDRESS = "dsh-resource://file/session/s1/src/move%09new.rs";

// On the wire this is the display of a single invalid 0xFF byte. It has the
// same characters as LITERAL_RAW_PATH, which is why decoding must reject it
// instead of opening a substituted literal-backslash file.
const INVALID_DISPLAY_PATH = String.raw`src/literal\xFF.rs`;
const INVALID_OLD_DISPLAY_PATH = String.raw`src/old\xFF.rs`;
const INVALID_NEW_DISPLAY_PATH = String.raw`src/new\xFE.rs`;

// Legacy show paths are raw valid UTF-8. This literal filename contains the
// three characters `\x41`; interpreting it with the new display decoder would
// wrongly turn it into the distinct file `src/litA.rs`.
const LEGACY_LITERAL_DISPLAY_PATH = String.raw`src/lit\\x41.rs`;
const LEGACY_LITERAL_RAW_PATH = String.raw`src/lit\x41.rs`;
const LEGACY_HIGH_DISPLAY_PATH = String.raw`src/lit\\x80.rs`;
const LEGACY_HIGH_RAW_PATH = String.raw`src/lit\x80.rs`;
const WRONG_FILE_PATH = "src/litA.rs";
const LEGACY_LITERAL_ADDRESS =
  "dsh-resource://file/session/s1/src/lit%5Cx41.rs";

const CODECT = process.env.CODECT_BIN;
if (!CODECT) {
  throw new Error(
    "CODECT_BIN must point at a built codect binary; " +
      "run `just test-dsh` or set CODECT_BIN=<path>"
  );
}

const DiffResult = TYPERT.invocations.find(
  (invocation) => invocation.id === "@nixypanda/dsh-codect#codect/diff"
).result.create();
const ShowResult = TYPERT.invocations.find(
  (invocation) => invocation.id === "@nixypanda/dsh-codect#codect/show"
).result.create();

function assertMarkedShow(doc) {
  assert.equal(doc.schema, "codect.show.v1");
  assert.equal(doc.path_encoding, "codect.path.v1");
  assert.equal(showNavigationEncoding(doc), "codect.path.v1");
  assert.equal(ShowResult.safeParse(doc).success, true);
}

function runCodect(args, cwd) {
  const output = execFileSync(CODECT, args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 50 * 1024 * 1024,
    timeout: 30000,
  });
  return JSON.parse(output);
}

function runCodectText(args, cwd) {
  return execFileSync(CODECT, args, {
    cwd,
    encoding: "utf8",
    maxBuffer: 50 * 1024 * 1024,
    timeout: 30000,
  });
}

function git(dir, ...args) {
  execFileSync("git", args, { cwd: dir, stdio: "ignore" });
}

function gitOutput(dir, ...args) {
  return execFileSync("git", args, { cwd: dir, encoding: "utf8" }).trim();
}

function write(dir, path, text) {
  const full = join(dir, path);
  mkdirSync(dirname(full), { recursive: true });
  writeFileSync(full, text);
}

function makeMoveFixture() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-move-"));
  git(dir, "init", "-q");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");

  write(dir, "src/a.rs", "pub struct Alpha { pub x: i32 }\n");
  write(
    dir,
    "src/removed.rs",
    "pub struct Removed { pub gone: bool, pub lost: bool }\n"
  );
  write(dir, "src/exact_old.rs", "pub struct Exact { pub kept: u8 }\n");
  write(dir, "src/edited_old.rs", "pub struct Edited { pub value: i32 }\n");
  write(
    dir,
    "src/body_old.rs",
    "pub struct Body { pub kept: u8 }\n\n" +
      "pub fn value() -> i32 {\n    1\n}\n"
  );
  write(
    dir,
    "src/decl_old.rs",
    "pub struct Decl { pub keep: u8, pub drop_me: bool }\n"
  );

  write(
    dir,
    "lang/old/Point.elm",
    "module Point exposing (Point)\n\ntype alias Point =\n    { x : Int }\n"
  );
  write(
    dir,
    "lang/old/Point.hs",
    "module Point (Point(..)) where\n\ndata Point = Point { x :: Int }\n"
  );
  write(dir, "lang/old/point.py", "class Point:\n    x: int\n");
  write(dir, "lang/old/point.rs", "pub struct Point { pub x: i32 }\n");

  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "base");

  write(dir, "src/a.rs", "pub struct Alpha { pub x: i64 }\n");
  write(dir, "src/added.rs", "pub enum NewThing { One, Two }\n");
  rmSync(join(dir, "src/removed.rs"));

  git(dir, "mv", "src/exact_old.rs", "src/exact_new.rs");

  git(dir, "mv", "src/edited_old.rs", "src/edited_new.rs");
  write(dir, "src/edited_new.rs", "pub struct Edited { pub value: i64 }\n");

  git(dir, "mv", "src/body_old.rs", "src/body_new.rs");
  write(
    dir,
    "src/body_new.rs",
    "pub struct Body { pub kept: u8 }\n\n" +
      "pub fn value() -> i32 {\n    2\n}\n"
  );

  git(dir, "mv", "src/decl_old.rs", "src/decl_new.rs");
  write(dir, "src/decl_new.rs", "pub struct Decl { pub keep: u8 }\n");

  mkdirSync(join(dir, "lang/new"), { recursive: true });
  git(dir, "mv", "lang/old/point.rs", "lang/new/point.rs");
  git(dir, "mv", "lang/old/point.py", "lang/new/point.py");
  git(dir, "mv", "lang/old/Point.elm", "lang/new/Point.elm");
  git(dir, "mv", "lang/old/Point.hs", "lang/new/Point.hs");

  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "moves and edits");
  return dir;
}

// The documented source-content score can prefer a shorter, unrelated file
// that shares language boilerplate over the semantically expected rename. This
// fixture pins the mandated descending-score outcome, not a "true move": DSH
// must consume the producer's endpoints rather than re-pair them.
function makeAmbiguousScoreFixture() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-ambiguous-"));
  git(dir, "init", "-q");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");

  write(dir, "src/deleted.rs", "pub struct Deleted { pub z: bool }\n");
  write(
    dir,
    "src/decl_old.rs",
    "pub struct Decl { pub keep: u8, pub drop_me: bool }\n"
  );
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "base");

  rmSync(join(dir, "src/deleted.rs"));
  git(dir, "mv", "src/decl_old.rs", "src/decl_new.rs");
  write(dir, "src/decl_new.rs", "pub struct Decl { pub keep: u8 }\n");
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "target");
  return dir;
}

// Real checkout names: backslash, newline, TAB, C0 control, and Unicode are all
// writable on POSIX and must survive codect's injective display encoding.
function makeEncodedPathFixture() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-encoded-"));
  git(dir, "init", "-q");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");

  const baseContent = "pub struct Same;\n";
  const targetContent = "pub struct Same { pub v: i64 }\n";
  for (const { raw } of ENCODED_PATHS) write(dir, raw, baseContent);
  write(dir, LITERAL_RAW_PATH, baseContent);
  write(dir, MOVE_OLD_RAW_PATH, "pub struct MovedControl;\n");
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "base");
  const base = gitOutput(dir, "rev-parse", "HEAD");

  for (const { raw } of ENCODED_PATHS) write(dir, raw, targetContent);
  write(dir, LITERAL_RAW_PATH, targetContent);
  rmSync(join(dir, MOVE_OLD_RAW_PATH));
  write(dir, MOVE_NEW_RAW_PATH, "pub struct MovedControl;\n");
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "target");
  const target = gitOutput(dir, "rev-parse", "HEAD");

  return { dir, base, target };
}

// Invalid UTF-8 filenames cannot be represented as JavaScript strings, so this
// fixture writes a raw git tree with `mktree -z`. The literal-backslash file and
// the 0xFF file share the same declaration name, making any display/stable-key
// collision visible.
function makeInvalidObjectTreeFixture() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-object-"));
  git(dir, "init", "-q");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");

  const hash = (content) =>
    execFileSync("git", ["hash-object", "-w", "--stdin"], {
      cwd: dir,
      input: Buffer.from(content),
    })
      .toString()
      .trim();
  const entry = (mode, type, sha, name) =>
    Buffer.concat([
      Buffer.from(`${mode} ${type} ${sha}\t`),
      name,
      Buffer.from([0]),
    ]);
  const tree = (entries) =>
    execFileSync("git", ["mktree", "-z"], {
      cwd: dir,
      input: Buffer.concat(entries),
    })
      .toString()
      .trim();
  const commit = (treeId, parent) => {
    const args = ["commit-tree", treeId];
    if (parent !== undefined) args.push("-p", parent);
    args.push("-m", "object tree");
    return gitOutput(dir, ...args);
  };

  const literalName = Buffer.from("literal\\xFF.rs");
  const invalidName = Buffer.concat([
    Buffer.from("literal"),
    Buffer.from([0xff]),
    Buffer.from(".rs"),
  ]);
  const oldInvalidName = Buffer.concat([
    Buffer.from("old"),
    Buffer.from([0xff]),
    Buffer.from(".rs"),
  ]);
  const newInvalidName = Buffer.concat([
    Buffer.from("new"),
    Buffer.from([0xfe]),
    Buffer.from(".rs"),
  ]);

  const same = "pub struct Same;\n";
  const changed = "pub struct Same { pub v: i64 }\n";
  const moves = "pub struct Moves;\n";
  const src1 = tree([
    entry("100644", "blob", hash(same), literalName),
    entry("100644", "blob", hash(same), invalidName),
    entry("100644", "blob", hash(moves), oldInvalidName),
  ]);
  const tree1 = tree([entry("040000", "tree", src1, Buffer.from("src"))]);
  const base = commit(tree1);

  const src2 = tree([
    entry("100644", "blob", hash(changed), literalName),
    entry("100644", "blob", hash(changed), invalidName),
    entry("100644", "blob", hash(moves), newInvalidName),
  ]);
  const tree2 = tree([entry("040000", "tree", src2, Buffer.from("src"))]);
  const target = commit(tree2, base);

  return { dir, base, target };
}

// New-producer Show paths and a marker-less legacy Show share v1. The literal
// `\x41` name is deliberately paired with the naive decode target `litA.rs`
// so selecting the wrong file is observable.
function makeLegacyShowFixture() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-legacy-show-"));
  git(dir, "init", "-q");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");

  write(dir, LEGACY_LITERAL_RAW_PATH, "pub struct LiteralEscape;\n");
  write(dir, LEGACY_HIGH_RAW_PATH, "pub struct LiteralHighEscape;\n");
  write(dir, WRONG_FILE_PATH, "pub struct PlainA;\n");
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "base");
  return { dir, revision: gitOutput(dir, "rev-parse", "HEAD") };
}

const fixture = makeMoveFixture();
const ambiguousFixture = makeAmbiguousScoreFixture();
const encodedFixture = makeEncodedPathFixture();
const objectTreeFixture = makeInvalidObjectTreeFixture();
const legacyShowFixture = makeLegacyShowFixture();
after(() => {
  rmSync(fixture, { recursive: true, force: true });
  rmSync(ambiguousFixture, { recursive: true, force: true });
  rmSync(encodedFixture.dir, { recursive: true, force: true });
  rmSync(objectTreeFixture.dir, { recursive: true, force: true });
  rmSync(legacyShowFixture.dir, { recursive: true, force: true });
});

function diff(mode, extra = []) {
  return runCodect(
    ["diff", "--format", "json", "--mode", mode, ...extra, "HEAD~1", "HEAD"],
    fixture
  );
}

function byPath(doc) {
  return new Map(doc.files.map((file) => [file.path, file]));
}

const typesDoc = diff("types");
const signaturesDoc = diff("signatures");
const typesByPath = byPath(typesDoc);

test("exact, edited, and body-only moves carry target path, old_path, and projection equality", () => {
  assert.equal(typesDoc.schema, "codect.diff.v2");

  const exact = typesByPath.get("src/exact_new.rs");
  assert.equal(exact?.status, "moved");
  assert.equal(exact?.old_path, "src/exact_old.rs");
  assert.equal(exact?.equal, true);
  assert.notEqual(exact?.base, null);
  assert.notEqual(exact?.target, null);
  assert.equal(exact?.base.projection.text, exact?.target.projection.text);

  const edited = typesByPath.get("src/edited_new.rs");
  assert.equal(edited?.status, "moved");
  assert.equal(edited?.old_path, "src/edited_old.rs");
  assert.equal(edited?.equal, false);
  assert.match(edited?.base.projection.text ?? "", /i32/);
  assert.match(edited?.target.projection.text ?? "", /i64/);

  const body = typesByPath.get("src/body_new.rs");
  assert.equal(body?.status, "moved");
  assert.equal(body?.old_path, "src/body_old.rs");
  assert.equal(body?.equal, true);
  assert.equal(body?.base.projection.text, body?.target.projection.text);
});

test("body-only move text has rename headers and no focused hunks", () => {
  const exact = runCodectText(
    [
      "diff",
      "--color",
      "never",
      "--mode",
      "signatures",
      "--path",
      "src/exact_new.rs",
      "HEAD~1",
      "HEAD",
    ],
    fixture
  );
  assert.equal(
    exact,
    "diff --codect a/src/exact_old.rs b/src/exact_new.rs\n" +
      "rename from src/exact_old.rs\n" +
      "rename to src/exact_new.rs\n"
  );

  const body = runCodectText(
    [
      "diff",
      "--color",
      "never",
      "--mode",
      "signatures",
      "--path",
      "src/body_new.rs",
      "HEAD~1",
      "HEAD",
    ],
    fixture
  );
  assert.match(body, /^diff --codect a\/src\/body_old\.rs b\/src\/body_new\.rs\n/);
  assert.match(body, /^rename from src\/body_old\.rs$/m);
  assert.match(body, /^rename to src\/body_new\.rs$/m);
  assert.equal(body.includes("@@"), false);
  assert.equal(body.includes("--- a/"), false);
});

test("an edited move keeps rename headers and shows only projected hunks", () => {
  const text = runCodectText(
    [
      "diff",
      "--color",
      "never",
      "--mode",
      "signatures",
      "--path",
      "src/edited_new.rs",
      "HEAD~1",
      "HEAD",
    ],
    fixture
  );
  assert.match(text, /^diff --codect a\/src\/edited_old\.rs b\/src\/edited_new\.rs\n/);
  assert.match(text, /^rename from src\/edited_old\.rs$/m);
  assert.match(text, /^rename to src\/edited_new\.rs$/m);
  assert.match(text, /^--- a\/src\/edited_old\.rs$/m);
  assert.match(text, /^\+\+\+ b\/src\/edited_new\.rs$/m);
  assert.match(text, /^-    pub value: i32,$/m);
  assert.match(text, /^\+    pub value: i64,$/m);
});

test("a scope naming either side returns the same whole move event", () => {
  const cases = [
    ["src/exact_old.rs", "src/exact_new.rs"],
    ["src/exact_new.rs", "src/exact_new.rs"],
    ["src/edited_old.rs", "src/edited_new.rs"],
    ["src/edited_new.rs", "src/edited_new.rs"],
    ["src/body_old.rs", "src/body_new.rs"],
    ["src/body_new.rs", "src/body_new.rs"],
  ];

  for (const [selected, target] of cases) {
    for (const mode of ["types", "signatures"]) {
      const doc = diff(mode, ["--path", selected]);
      assert.equal(doc.files.length, 1, `${mode} ${selected}`);
      const [move] = doc.files;
      assert.equal(move.status, "moved", `${mode} ${selected}`);
      assert.equal(move.path, target, `${mode} ${selected}`);
      assert.notEqual(move.base, null, `${mode} ${selected}`);
      assert.notEqual(move.target, null, `${mode} ${selected}`);
      assert.equal(move.equal, move.base.projection.text === move.target.projection.text);
    }
  }
});

test("non-move events never carry old_path and keep the v2 side shape", () => {
  for (const file of typesDoc.files) {
    if (file.status === "moved") continue;
    assert.equal("old_path" in file, false, file.path);
    assert.equal(file.equal, false, file.path);
    if (file.status === "added") {
      assert.equal(file.base, null);
      assert.notEqual(file.target, null);
    } else if (file.status === "deleted") {
      assert.notEqual(file.base, null);
      assert.equal(file.target, null);
    } else {
      assert.equal(file.status, "modified");
      assert.notEqual(file.base, null);
      assert.notEqual(file.target, null);
    }
  }
  assert.equal(typesByPath.get("src/added.rs")?.status, "added");
  assert.equal(typesByPath.get("src/removed.rs")?.status, "deleted");
  assert.equal(typesByPath.get("src/a.rs")?.status, "modified");
});

test("the host DiffResult accepts real CLI documents in both requested modes", () => {
  for (const doc of [typesDoc, signaturesDoc]) {
    const result = DiffResult.safeParse(doc);
    assert.equal(result.success, true, JSON.stringify(result.error?.issues));
  }
});

test("the host DiffResult rejects v1 and malformed move documents at runtime", () => {
  const exact = typesByPath.get("src/exact_new.rs");
  const modified = typesByPath.get("src/a.rs");

  const reject = (name, doc) => {
    assert.equal(DiffResult.safeParse(doc).success, false, name);
  };

  reject("v1 schema", { ...typesDoc, schema: "codect.diff.v1" });
  reject("unknown schema", { ...typesDoc, schema: "codect.diff.v3" });
  reject("move with identical paths", {
    ...typesDoc,
    files: [{ ...exact, path: exact.old_path }],
  });

  const { old_path: _omitted, ...withoutOldPath } = exact;
  reject("move without old_path", { ...typesDoc, files: [withoutOldPath] });
  reject("move with null old_path", {
    ...typesDoc,
    files: [{ ...exact, old_path: null }],
  });
  reject("move with inconsistent equal", {
    ...typesDoc,
    files: [{ ...exact, equal: !exact.equal }],
  });
  reject("move with a null side", {
    ...typesDoc,
    files: [{ ...exact, base: null }],
  });
  reject("non-move carrying old_path", {
    ...typesDoc,
    files: [{ ...modified, old_path: "src/a.rs" }],
  });
  reject("non-move equal: true", {
    ...typesDoc,
    files: [{ ...modified, equal: true }],
  });
});

test("tree rows are keyed by the target path and old paths do not become rows", () => {
  const rows = buildTreeRows(
    typesDoc.files.map((file) => file.path),
    new Set()
  );
  const fileRows = rows.filter((row) => row.kind === "file").map((row) => row.path);

  for (const target of [
    "src/exact_new.rs",
    "src/edited_new.rs",
    "src/body_new.rs",
    "src/decl_new.rs",
  ]) {
    assert.ok(fileRows.includes(target), target);
    assert.equal(
      fileRows.includes(target.replace("_new.rs", "_old.rs")),
      false,
      target
    );
  }

  const exact = typesByPath.get("src/exact_new.rs");
  assert.equal(movedFrom(exact), "src/exact_old.rs");
  assert.equal(basePath(exact), "src/exact_old.rs");
  assert.equal(targetPath(exact), "src/exact_new.rs");
  assert.equal(moveLabel(exact), "src/exact_old.rs → src/exact_new.rs");

  const deleted = typesByPath.get("src/removed.rs");
  assert.equal(basePath(deleted), "src/removed.rs");
  assert.equal(targetPath(deleted), null);
  assert.equal(movedFrom(deleted), undefined);
});

test("a deleted declaration inside a move is rebased and gated non-navigable", () => {
  const decl = typesByPath.get("src/decl_new.rs");
  assert.equal(decl?.status, "moved");
  assert.equal(decl?.old_path, "src/decl_old.rs");
  assert.equal(decl?.equal, false);

  const changes = diffChanges(decl);
  const deleted = changes.filter((change) => change.status === "deleted");
  assert.equal(deleted.length, 1);
  assert.equal(
    deleted[0].item.stable_key,
    "src/decl_new.rs::type::Decl::field::drop_me"
  );
  assert.equal(canNavigateChange(decl, "deleted"), false);
  assert.equal(pathForChange(decl, "deleted"), "src/decl_old.rs");

  const edited = typesByPath.get("src/edited_new.rs");
  const changed = diffChanges(edited).filter((change) => change.status === "changed");
  assert.ok(changed.length >= 1);
  assert.equal(canNavigateChange(edited, "changed"), true);
  assert.equal(pathForChange(edited, "changed"), "src/edited_new.rs");
});

test("stable-key rebasing suppresses false declaration changes in every language", () => {
  const allMoves = typesDoc.files.filter(
    (file) =>
      file.status === "moved" &&
      file.old_path.startsWith("lang/old/") &&
      file.path.startsWith("lang/new/")
  );
  const languages = new Set(allMoves.map((file) => file.language));
  for (const language of ["elm", "haskell", "python", "rust"]) {
    assert.ok(languages.has(language), language);
  }

  for (const file of allMoves) {
    assert.deepEqual(
      diffChanges(file),
      [],
      `${file.language}: ${file.old_path} -> ${file.path}`
    );
  }

  for (const file of allMoves) {
    if (file.language !== "python" && file.language !== "rust") continue;
    assert.notEqual(
      file.base.outline[0]?.stable_key,
      file.target.outline[0]?.stable_key,
      `${file.language} source keys should be path-prefixed`
    );
  }
});

test("ambiguous boilerplate follows the documented score order and consumers keep producer endpoints", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD"],
    ambiguousFixture
  );
  const files = byPath(doc);
  const move = files.get("src/decl_new.rs");
  const deleted = files.get("src/decl_old.rs");

  // Under the mandated scalar source score, `src/deleted.rs` (shorter, shared
  // boilerplate) outranks the true edited rename; descending greedy takes it.
  assert.equal(move?.status, "moved");
  assert.equal(move?.old_path, "src/deleted.rs");
  assert.equal(move?.equal, false);
  assert.equal(deleted?.status, "deleted");
  assert.equal(DiffResult.safeParse(doc).success, true);

  // The consumer contract is endpoint fidelity, not re-pairing.
  assert.equal(basePath(move), "src/deleted.rs");
  assert.equal(targetPath(move), "src/decl_new.rs");
  assert.equal(moveLabel(move), "src/deleted.rs → src/decl_new.rs");
  const rows = buildTreeRows(
    doc.files.map((file) => file.path),
    new Set()
  )
    .filter((row) => row.kind === "file")
    .map((row) => row.path);
  assert.ok(rows.includes("src/decl_new.rs"));
  assert.equal(rows.includes("src/deleted.rs"), false);

  // Stable-key rebasing follows old_path even when it is not the semantic
  // source the user might expect.
  const changes = diffChanges(move);
  assert.deepEqual(
    changes.map((change) => change.status).sort(),
    ["added", "deleted"]
  );
  for (const change of changes) {
    assert.ok(change.item.stable_key.startsWith("src/decl_new.rs::"));
  }
});

test("Show and Diff decode backslash, control, and Unicode display paths to real names", () => {
  const showBase = runCodect(
    ["show", "--format", "json", "--mode", "types", encodedFixture.base],
    encodedFixture.dir
  );
  const showTarget = runCodect(
    ["show", "--format", "json", "--mode", "types", encodedFixture.target],
    encodedFixture.dir
  );
  const diffDoc = runCodect(
    [
      "diff",
      "--format",
      "json",
      "--mode",
      "types",
      encodedFixture.base,
      encodedFixture.target,
    ],
    encodedFixture.dir
  );
  assertMarkedShow(showBase);
  assertMarkedShow(showTarget);
  const showEncoding = showNavigationEncoding(showBase);
  const baseByDisplay = byPath(showBase);
  const targetByDisplay = byPath(showTarget);
  const diffByDisplay = byPath(diffDoc);

  for (const { raw, display, address } of ENCODED_PATHS) {
    const file = baseByDisplay.get(display);
    assert.ok(file !== undefined, display);
    const resolved = navigationPath(file.path, showEncoding);
    assert.equal(resolved, raw);
    assert.equal(repoFileAddress("s1", resolved), address);
    assert.equal(file.outline[0].stable_key, `${display}::type::Same`);

    const modified = diffByDisplay.get(display);
    assert.equal(modified?.status, "modified", display);
    assert.equal(navigationPath(modified.path, DIFF_PATH_ENCODING), raw);
    assert.equal(modified.base.outline[0].stable_key, `${display}::type::Same`);
  }

  const literal = baseByDisplay.get(LITERAL_DISPLAY_PATH);
  assert.ok(literal !== undefined);
  const literalPath = navigationPath(literal.path, showEncoding);
  assert.equal(literalPath, LITERAL_RAW_PATH);
  assert.equal(repoFileAddress("s1", literalPath), LITERAL_ADDRESS);
  assert.equal(
    literal.outline[0].stable_key,
    `${LITERAL_DISPLAY_PATH}::type::Same`
  );

  const literalDiff = diffByDisplay.get(LITERAL_DISPLAY_PATH);
  assert.equal(literalDiff?.status, "modified");
  assert.equal(
    navigationPath(literalDiff.path, DIFF_PATH_ENCODING),
    LITERAL_RAW_PATH
  );

  const move = diffByDisplay.get(MOVE_NEW_DISPLAY_PATH);
  assert.equal(move?.status, "moved");
  assert.equal(move?.old_path, MOVE_OLD_DISPLAY_PATH);
  const moveNewPath = navigationPath(move.path, DIFF_PATH_ENCODING);
  const moveOldPath = navigationPath(move.old_path, DIFF_PATH_ENCODING);
  assert.equal(moveNewPath, MOVE_NEW_RAW_PATH);
  assert.equal(moveOldPath, MOVE_OLD_RAW_PATH);
  assert.equal(repoFileAddress("s1", moveNewPath), MOVE_NEW_ADDRESS);
  assert.equal(repoFileAddress("s1", moveOldPath), MOVE_OLD_ADDRESS);
  assert.ok(
    move.base.outline[0].stable_key.startsWith(`${MOVE_OLD_DISPLAY_PATH}::`)
  );
  assert.ok(
    move.target.outline[0].stable_key.startsWith(`${MOVE_NEW_DISPLAY_PATH}::`)
  );
  assert.deepEqual(diffChanges(move), []);
  assert.ok(targetByDisplay.has(MOVE_NEW_DISPLAY_PATH));
  assert.equal(targetByDisplay.has(MOVE_OLD_DISPLAY_PATH), false);

  // Display paths and stable keys stay distinct and never contain the raw
  // controls they represent.
  const displayPaths = showBase.files.map((file) => file.path);
  assert.equal(new Set(displayPaths).size, showBase.files.length);
  const stableKeys = showBase.files.map((file) => file.outline[0].stable_key);
  assert.equal(new Set(stableKeys).size, showBase.files.length);
  for (const { display } of ENCODED_PATHS) {
    const key = baseByDisplay.get(display).outline[0].stable_key;
    assert.ok(key.startsWith(`${display}::`));
    assert.equal(key.includes("\n"), false);
    assert.equal(key.includes("\t"), false);
    assert.equal(key.includes("\x01"), false);
  }
});

test("literal \\xFF and invalid 0xFF paths stay distinct and invalid bytes decode null", () => {
  const showBase = runCodect(
    ["show", "--format", "json", "--mode", "types", objectTreeFixture.base],
    objectTreeFixture.dir
  );
  const showTarget = runCodect(
    ["show", "--format", "json", "--mode", "types", objectTreeFixture.target],
    objectTreeFixture.dir
  );
  const diffDoc = runCodect(
    [
      "diff",
      "--format",
      "json",
      "--mode",
      "types",
      objectTreeFixture.base,
      objectTreeFixture.target,
    ],
    objectTreeFixture.dir
  );
  assertMarkedShow(showBase);
  assertMarkedShow(showTarget);
  const showEncoding = showNavigationEncoding(showBase);
  const baseByDisplay = byPath(showBase);
  const targetByDisplay = byPath(showTarget);
  const diffByDisplay = byPath(diffDoc);

  const literal = baseByDisplay.get(LITERAL_DISPLAY_PATH);
  const invalid = baseByDisplay.get(INVALID_DISPLAY_PATH);
  assert.equal(literal?.path, LITERAL_DISPLAY_PATH);
  assert.equal(invalid?.path, INVALID_DISPLAY_PATH);
  assert.notEqual(LITERAL_DISPLAY_PATH, INVALID_DISPLAY_PATH);
  assert.equal(
    LITERAL_DISPLAY_PATH.length,
    INVALID_DISPLAY_PATH.length + 1
  );

  const literalPath = navigationPath(literal.path, showEncoding);
  assert.equal(literalPath, LITERAL_RAW_PATH);
  // navigateFrom is nested in client.apply and not exported; this null is the
  // gate it checks before building an address, so it is the accessible half of
  // the invalid-UTF-8 navigation contract.
  assert.equal(navigationPath(invalid.path, showEncoding), null);
  assert.equal(canNavigatePath(invalid.path, showEncoding), false);
  assert.equal(repoFileAddress("s1", literalPath), LITERAL_ADDRESS);

  assert.equal(
    literal.outline[0].stable_key,
    `${LITERAL_DISPLAY_PATH}::type::Same`
  );
  assert.equal(
    invalid.outline[0].stable_key,
    `${INVALID_DISPLAY_PATH}::type::Same`
  );
  assert.notEqual(
    literal.outline[0].stable_key,
    invalid.outline[0].stable_key
  );

  const displayPaths = showBase.files.map((file) => file.path);
  const stableKeys = showBase.files.map((file) => file.outline[0].stable_key);
  assert.equal(new Set(displayPaths).size, showBase.files.length);
  assert.equal(new Set(stableKeys).size, showBase.files.length);

  assert.ok(targetByDisplay.has(INVALID_NEW_DISPLAY_PATH));
  assert.equal(
    navigationPath(INVALID_NEW_DISPLAY_PATH, showEncoding),
    null
  );

  const literalDiff = diffByDisplay.get(LITERAL_DISPLAY_PATH);
  assert.equal(literalDiff?.status, "modified");
  assert.equal(
    navigationPath(literalDiff.path, DIFF_PATH_ENCODING),
    LITERAL_RAW_PATH
  );

  const invalidDiff = diffByDisplay.get(INVALID_DISPLAY_PATH);
  assert.equal(invalidDiff?.status, "modified");
  assert.equal(navigationPath(invalidDiff.path, DIFF_PATH_ENCODING), null);
  assert.equal(canNavigatePath(invalidDiff.path, DIFF_PATH_ENCODING), false);

  const move = diffByDisplay.get(INVALID_NEW_DISPLAY_PATH);
  assert.equal(move?.status, "moved");
  assert.equal(move?.old_path, INVALID_OLD_DISPLAY_PATH);
  assert.equal(navigationPath(move.path, DIFF_PATH_ENCODING), null);
  assert.equal(navigationPath(move.old_path, DIFF_PATH_ENCODING), null);
  assert.equal(canNavigatePath(move.path, DIFF_PATH_ENCODING), false);
  assert.equal(canNavigatePath(move.old_path, DIFF_PATH_ENCODING), false);
  assert.equal(move.equal, true);
  assert.ok(
    move.base.outline[0].stable_key.startsWith(`${INVALID_OLD_DISPLAY_PATH}::`)
  );
  assert.ok(
    move.target.outline[0].stable_key.startsWith(`${INVALID_NEW_DISPLAY_PATH}::`)
  );
  assert.deepEqual(diffChanges(move), []);
  assert.equal(DiffResult.safeParse(diffDoc).success, true);
});

test("new marked Show and legacy unmarked Show resolve literal escapes without selecting litA", () => {
  const current = runCodect(
    ["show", "--format", "json", "--mode", "types", legacyShowFixture.revision],
    legacyShowFixture.dir
  );
  assertMarkedShow(current);
  const currentEncoding = showNavigationEncoding(current);
  const currentByPath = byPath(current);
  const currentLiteral = currentByPath.get(LEGACY_LITERAL_DISPLAY_PATH);
  const highLiteral = currentByPath.get(LEGACY_HIGH_DISPLAY_PATH);
  assert.ok(currentLiteral !== undefined);
  assert.ok(highLiteral !== undefined);
  assert.ok(currentByPath.has(WRONG_FILE_PATH));
  assert.equal(
    navigationPath(currentLiteral.path, currentEncoding),
    LEGACY_LITERAL_RAW_PATH
  );

  // Same show v1 schema, no marker: this is what an old producer emitted for
  // the same literal-backslash filename. Mutate the real CLI document's path
  // to that explicit legacy raw string.
  const legacy = JSON.parse(JSON.stringify(current));
  delete legacy.path_encoding;
  assert.equal("path_encoding" in legacy, false);
  const legacyLiteral = legacy.files.find(
    (file) => file.path === LEGACY_LITERAL_DISPLAY_PATH
  );
  const legacyHigh = legacy.files.find(
    (file) => file.path === LEGACY_HIGH_DISPLAY_PATH
  );
  assert.ok(legacyLiteral !== undefined);
  assert.ok(legacyHigh !== undefined);
  legacyLiteral.path = LEGACY_LITERAL_RAW_PATH;
  legacyHigh.path = LEGACY_HIGH_RAW_PATH;

  assert.equal(legacy.schema, "codect.show.v1");
  assert.equal(ShowResult.safeParse(legacy).success, true);
  const legacyEncoding = showNavigationEncoding(legacy);
  assert.equal(legacyEncoding, "legacy");

  const legacyPath = navigationPath(legacyLiteral.path, legacyEncoding);
  assert.equal(legacyPath, LEGACY_LITERAL_RAW_PATH);
  assert.notEqual(legacyPath, WRONG_FILE_PATH);
  assert.equal(
    navigationPath(legacyLiteral.path, currentEncoding),
    WRONG_FILE_PATH,
    "the marked decoder would choose the wrong file; the marker must prevent this"
  );
  assert.equal(canNavigatePath(legacyLiteral.path, legacyEncoding), true);

  const legacyAddress = repoFileAddress("s1", legacyPath);
  assert.equal(legacyAddress, LEGACY_LITERAL_ADDRESS);
  assert.notEqual(
    legacyAddress,
    repoFileAddress("s1", WRONG_FILE_PATH),
    "navigation must not select src/litA.rs"
  );

  // A legacy `\x80` escape is ambiguous between a literal filename and an
  // invalid byte, so navigation is disabled rather than guessed.
  assert.equal(navigationPath(legacyHigh.path, legacyEncoding), null);
  assert.equal(canNavigatePath(legacyHigh.path, legacyEncoding), false);

  // Unknown markers are unsupported for path decoding; ShowResult also
  // rejects them because the optional property is a literal.
  const unknown = JSON.parse(JSON.stringify(current));
  unknown.path_encoding = "codect.path.v2";
  assert.equal(ShowResult.safeParse(unknown).success, false);
  const unknownEncoding = showNavigationEncoding(unknown);
  assert.equal(unknownEncoding, "unsupported");
  assert.equal(navigationPath(unknown.files[0].path, unknownEncoding), null);
  assert.equal(canNavigatePath(unknown.files[0].path, unknownEncoding), false);
});
