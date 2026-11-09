/**
 * Contract tests: `codect show/diff --format json` matches the shapes the DSH
 * plugin consumes (docs/schema/codect.{show,diff}.v1/v2.json).
 *
 * The diff tests build a throwaway repository so added/deleted/modified/moved
 * files are always present — a HEAD~1..HEAD diff of this repo can legitimately
 * be empty, which previously let a wrong schema pass unnoticed.
 */

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";
import { diffChanges } from "../src/client-codect/declaration-diff.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const codectRoot = join(__dirname, "../../..");

const CODECT = process.env.CODECT_BIN;
if (!CODECT) {
  throw new Error(
    "CODECT_BIN must point at a built codect binary; " +
      "run `just test-dsh` or set CODECT_BIN=<path>"
  );
}

function runCodect(args, cwd) {
  const output = execFileSync(CODECT, args, {
    cwd: cwd ?? codectRoot,
    encoding: "utf8",
    maxBuffer: 50 * 1024 * 1024,
    timeout: 30000,
  });
  return JSON.parse(output);
}

function makeFixtureRepo() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, stdio: "ignore" });
  git("init", "-q");
  git("config", "user.email", "test@example.com");
  git("config", "user.name", "test");
  mkdirSync(join(dir, "src"));

  writeFileSync(join(dir, "src/a.rs"), "pub struct Alpha { pub x: i32 }\n");
  writeFileSync(join(dir, "src/b.rs"), "pub struct Obsolete { pub old: bool }\n");
  writeFileSync(join(dir, "src/old_exact.rs"), "pub struct Renamed { pub kept: u8 }\n");
  writeFileSync(join(dir, "src/old_edited.rs"), "pub struct Edited { pub value: i32 }\n");
  git("add", "-A");
  git("commit", "-qm", "one");

  writeFileSync(join(dir, "src/a.rs"), "pub struct Alpha { pub x: i64 }\n");
  writeFileSync(join(dir, "src/c.rs"), "pub enum NewThing { One, Two }\n");
  rmSync(join(dir, "src/b.rs"));
  rmSync(join(dir, "src/old_exact.rs"));
  writeFileSync(join(dir, "src/new_exact.rs"), "pub struct Renamed { pub kept: u8 }\n");
  rmSync(join(dir, "src/old_edited.rs"));
  writeFileSync(join(dir, "src/new_edited.rs"), "pub struct Edited { pub value: i64 }\n");
  git("add", "-A");
  git("commit", "-qm", "two");

  return dir;
}

const fixture = makeFixtureRepo();
after(() => rmSync(fixture, { recursive: true, force: true }));

function makeSnapshotRepo() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-snap-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, stdio: "ignore" });
  git("init", "-q");
  git("config", "user.email", "test@example.com");
  git("config", "user.name", "test");
  mkdirSync(join(dir, "src"));

  writeFileSync(join(dir, "src/tracked.rs"), "pub struct A { pub x: i32 }\n");
  git("add", "-A");
  git("commit", "-qm", "one");

  writeFileSync(join(dir, "src/tracked.rs"), "pub struct A { pub x: i64 }\n");
  git("add", "src/tracked.rs");
  writeFileSync(join(dir, "src/tracked.rs"), "pub struct A { pub x: u64 }\n");

  writeFileSync(join(dir, "src/untracked.rs"), "pub struct U { pub y: i32 }\n");
  writeFileSync(join(dir, ".gitignore"), "ignored.rs\n");
  writeFileSync(join(dir, "src/ignored.rs"), "pub struct I { pub z: i32 }\n");

  return dir;
}

const snapshotFixture = makeSnapshotRepo();
after(() => rmSync(snapshotFixture, { recursive: true, force: true }));

let worktreeSupport;
function worktreeUnsupportedReason() {
  if (worktreeSupport === undefined) {
    try {
      const doc = runCodect(
        ["diff", "--format", "json", "--mode", "types", "HEAD", ":worktree"],
        snapshotFixture
      );
      worktreeSupport = doc.files.some((f) => f.path === "src/untracked.rs");
    } catch {
      worktreeSupport = false;
    }
  }
  return worktreeSupport
    ? false
    : "codect binary lacks untracked-:worktree support; run `just test-dsh` or point CODECT_BIN at a current build";
}

test("codect show returns a codect.show.v1 document", () => {
  const doc = runCodect(["show", "--format", "json", "--mode", "types", "HEAD"]);
  assert.equal(doc.schema, "codect.show.v1");
  assert.ok(["revision", "stdin", "worktree"].includes(doc.input));
  assert.ok(typeof doc.revision === "string");
  assert.ok(["types", "signatures"].includes(doc.mode));
  assert.ok(Array.isArray(doc.files));
  for (const file of doc.files) {
    assert.ok(typeof file.path === "string");
    assert.ok(typeof file.language === "string");
    assert.ok(typeof file.projection.text === "string");
    assert.ok(Array.isArray(file.projection.items));
    assert.ok(Array.isArray(file.outline));
    for (const item of file.projection.items) {
      assert.ok(typeof item.stable_key === "string");
      assert.ok(item.parent_key === null || typeof item.parent_key === "string");
      assert.ok(typeof item.canonical_text === "string");
      assert.ok(typeof item.span.start_line === "number");
    }
  }
});

test("codect show --mode signatures includes signatures", () => {
  const doc = runCodect(["show", "--format", "json", "--mode", "signatures", "HEAD"]);
  assert.equal(doc.schema, "codect.show.v1");
  assert.equal(doc.mode, "signatures");
});

test("codect diff returns the side-based codect.diff.v2 document", () => {
  const doc = runCodect(["diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD"], fixture);

  assert.equal(doc.schema, "codect.diff.v2");
  assert.ok(["types", "signatures"].includes(doc.mode));
  assert.ok(typeof doc.base.revision === "string");
  assert.ok(typeof doc.target.revision === "string");
  assert.ok(Array.isArray(doc.files));

  const statuses = new Set();
  for (const file of doc.files) {
    assert.ok(typeof file.path === "string");
    assert.ok(typeof file.language === "string");
    assert.ok(
      ["added", "deleted", "modified", "moved"].includes(file.status)
    );
    statuses.add(file.status);
    for (const side of [file.base, file.target]) {
      if (side === null) continue;
      assert.ok(typeof side.snapshot_id === "string");
      assert.ok(typeof side.projection.text === "string");
      assert.ok(Array.isArray(side.projection.items));
      assert.ok(Array.isArray(side.outline));
    }
    if (file.status === "moved") {
      assert.ok(typeof file.old_path === "string");
      assert.notEqual(file.old_path, file.path);
      assert.notEqual(file.base, null);
      assert.notEqual(file.target, null);
      assert.equal(typeof file.equal, "boolean");
      assert.equal(
        file.equal,
        file.base.projection.text === file.target.projection.text
      );
    } else {
      assert.equal("old_path" in file, false);
      assert.equal(file.equal, false);
      if (file.status === "added") {
        assert.equal(file.base, null);
        assert.notEqual(file.target, null);
      } else if (file.status === "deleted") {
        assert.notEqual(file.base, null);
        assert.equal(file.target, null);
      } else {
        assert.notEqual(file.base, null);
        assert.notEqual(file.target, null);
      }
    }
    assert.ok(!("items" in file), "diff files do not carry a top-level items array");
  }
  assert.deepEqual(
    [...statuses].sort(),
    ["added", "deleted", "modified", "moved"]
  );

  const byPath = new Map(doc.files.map((file) => [file.path, file]));
  const exact = byPath.get("src/new_exact.rs");
  assert.equal(exact?.status, "moved");
  assert.equal(exact?.old_path, "src/old_exact.rs");
  assert.equal(exact?.equal, true);

  const edited = byPath.get("src/new_edited.rs");
  assert.equal(edited?.status, "moved");
  assert.equal(edited?.old_path, "src/old_edited.rs");
  assert.equal(edited?.equal, false);
  assert.notEqual(
    edited?.base.projection.text,
    edited?.target.projection.text
  );
});

test("client outline comparison yields added/deleted/changed declarations", () => {
  const doc = runCodect(["diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD"], fixture);
  const byPath = new Map(doc.files.map((f) => [f.path, f]));

  const modified = diffChanges(byPath.get("src/a.rs"));
  assert.ok(modified.length >= 1);
  assert.ok(modified.every((c) => c.status === "changed"));
  assert.ok(modified.some((c) => c.item.name === "x"));

  const deleted = diffChanges(byPath.get("src/b.rs"));
  assert.ok(deleted.length >= 1);
  assert.ok(deleted.every((c) => c.status === "deleted"));

  const added = diffChanges(byPath.get("src/c.rs"));
  assert.ok(added.length >= 1);
  assert.ok(added.every((c) => c.status === "added"));

  const exactMove = byPath.get("src/new_exact.rs");
  assert.ok(exactMove !== undefined);
  assert.deepEqual(diffChanges(exactMove), []);
});

test("codect diff with --path filter", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "signatures", "--path", "src/c.rs", "HEAD~1", "HEAD"],
    fixture
  );
  assert.equal(doc.schema, "codect.diff.v2");
  assert.equal(doc.files.length, 1);
  assert.equal(doc.files[0].path, "src/c.rs");
});

test("a scope selecting either path returns the whole move event", () => {
  for (const selected of ["src/old_exact.rs", "src/new_exact.rs"]) {
    const doc = runCodect(
      ["diff", "--format", "json", "--mode", "types", "--path", selected, "HEAD~1", "HEAD"],
      fixture
    );
    assert.equal(doc.schema, "codect.diff.v2");
    assert.equal(doc.files.length, 1);
    const [move] = doc.files;
    assert.equal(move.status, "moved");
    assert.equal(move.path, "src/new_exact.rs");
    assert.equal(move.old_path, "src/old_exact.rs");
    assert.notEqual(move.base, null);
    assert.notEqual(move.target, null);
    assert.equal(move.equal, true);
  }
});

test("codect show :worktree reads from worktree", () => {
  const doc = runCodect([
    "show",
    "--format",
    "json",
    "--mode",
    "types",
    "--worktree",
    "--path",
    "crates/base/src/lib.rs",
  ]);
  assert.equal(doc.schema, "codect.show.v1");
});

test(
  "codect diff HEAD :worktree includes modified and untracked files",
  { skip: worktreeUnsupportedReason() },
  () => {
    const doc = runCodect(
      ["diff", "--format", "json", "--mode", "types", "HEAD", ":worktree"],
      snapshotFixture
    );

    assert.equal(doc.schema, "codect.diff.v2");
    assert.equal(doc.base.kind, "commit");
    assert.equal(doc.target.kind, "worktree");
    assert.equal(doc.target.revision, ":worktree");
    assert.ok(typeof doc.target.id === "string" && doc.target.id.length > 0);

    const byPath = new Map(doc.files.map((f) => [f.path, f]));
    assert.equal(byPath.get("src/tracked.rs")?.status, "modified");
    assert.equal(byPath.get("src/untracked.rs")?.status, "added");
    assert.equal(byPath.has("src/ignored.rs"), false);
  }
);

test("codect diff :index :worktree isolates unstaged changes", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "types", ":index", ":worktree"],
    snapshotFixture
  );

  assert.equal(doc.base.kind, "index");
  assert.equal(doc.target.kind, "worktree");

  const tracked = doc.files.find((f) => f.path === "src/tracked.rs");
  assert.equal(tracked?.status, "modified");
  assert.ok(tracked.base.projection.text.includes("i64"));
  assert.ok(tracked.target.projection.text.includes("u64"));
});

test(
  "codect diff :empty :worktree marks every file added",
  { skip: worktreeUnsupportedReason() },
  () => {
    const doc = runCodect(
      ["diff", "--format", "json", "--mode", "types", ":empty", ":worktree"],
      snapshotFixture
    );

    assert.equal(doc.base.kind, "empty");
    assert.equal(doc.target.kind, "worktree");
    assert.ok(doc.files.length > 0);
    assert.ok(doc.files.every((f) => f.status === "added"));
    assert.ok(doc.files.some((f) => f.path === "src/untracked.rs"));
  }
);
