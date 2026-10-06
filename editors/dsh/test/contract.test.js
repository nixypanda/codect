/**
 * Contract tests: `codect show/diff --format json` matches the shapes the DSH
 * plugin consumes (docs/schema/codect.{show,diff}.v1.json).
 *
 * The diff tests build a throwaway repository so added/deleted/modified files
 * are always present — a HEAD~1..HEAD diff of this repo can legitimately be
 * empty, which previously let a wrong schema pass unnoticed.
 */

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const __dirname = dirname(fileURLToPath(import.meta.url));

// codect repo root: editors/dsh/test → ../../.. = codect root
const codectRoot = join(__dirname, "../../..");
const CODECT = process.env.CODECT_BIN || "codect";

function runCodect(args, cwd) {
  const output = execFileSync(CODECT, args, {
    cwd: cwd ?? codectRoot,
    encoding: "utf8",
    maxBuffer: 50 * 1024 * 1024,
    timeout: 30000,
  });
  return JSON.parse(output);
}

/**
 * A repository with one modified, one deleted, and one added Rust file between
 * its two commits.
 */
function makeFixtureRepo() {
  const dir = mkdtempSync(join(tmpdir(), "codect-dsh-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, stdio: "ignore" });
  git("init", "-q");
  git("config", "user.email", "test@example.com");
  git("config", "user.name", "test");
  mkdirSync(join(dir, "src"));

  writeFileSync(join(dir, "src/a.rs"), "pub struct A { pub x: i32 }\n");
  writeFileSync(join(dir, "src/b.rs"), "pub struct B { pub y: i32 }\n");
  git("add", "-A");
  git("commit", "-qm", "one");

  writeFileSync(join(dir, "src/a.rs"), "pub struct A { pub x: i64 }\n");
  writeFileSync(join(dir, "src/c.rs"), "pub struct C { pub z: i32 }\n");
  rmSync(join(dir, "src/b.rs"));
  git("add", "-A");
  git("commit", "-qm", "two");

  return dir;
}

const fixture = makeFixtureRepo();
after(() => rmSync(fixture, { recursive: true, force: true }));

/**
 * A repository whose index and worktree diverge from HEAD:
 * - `src/tracked.rs` is committed, changed and staged, then changed again, so
 *   `:index` and `:worktree` disagree.
 * - `src/untracked.rs` is never added, so only `:worktree` sees it.
 * - `src/ignored.rs` matches `.gitignore`, so `:worktree` must exclude it.
 */
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

/** Client-side outline comparison, mirrored from diff-view.tsx. */
function diffChanges(file) {
  const base = new Map((file.base?.outline ?? []).map((i) => [i.stable_key, i]));
  const target = new Map((file.target?.outline ?? []).map((i) => [i.stable_key, i]));
  const changes = [];
  for (const [key, item] of base) {
    if (!target.has(key)) changes.push({ status: "deleted", item });
  }
  for (const [key, item] of target) {
    if (!base.has(key)) changes.push({ status: "added", item });
  }
  for (const [key, item] of target) {
    const before = base.get(key);
    if (before !== undefined && before.signature !== item.signature) {
      changes.push({ status: "changed", item });
    }
  }
  return changes;
}

test("codect show returns a codect.show.v1 document", () => {
  const doc = runCodect(["show", "--format", "json", "--mode", "types", "HEAD"]);
  assert.equal(doc.schema, "codect.show.v1");
  assert.ok(["revision", "worktree", "index", "empty"].includes(doc.input));
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

test("codect diff returns the side-based codect.diff.v1 document", () => {
  const doc = runCodect(["diff", "--format", "json", "--mode", "types", "HEAD~1", "HEAD"], fixture);

  assert.equal(doc.schema, "codect.diff.v1");
  assert.ok(["types", "signatures"].includes(doc.mode));
  assert.ok(typeof doc.base.revision === "string");
  assert.ok(typeof doc.target.revision === "string");
  assert.ok(Array.isArray(doc.files));

  const statuses = new Set();
  for (const file of doc.files) {
    assert.ok(typeof file.path === "string");
    assert.ok(typeof file.language === "string");
    assert.ok(["added", "deleted", "modified"].includes(file.status));
    assert.equal(file.equal, false);
    statuses.add(file.status);
    // A changed file carries base/target sides with projection + outline (or null).
    for (const side of [file.base, file.target]) {
      if (side === null) continue;
      assert.ok(typeof side.snapshot_id === "string");
      assert.ok(typeof side.projection.text === "string");
      assert.ok(Array.isArray(side.projection.items));
      assert.ok(Array.isArray(side.outline));
      // `items` is not part of a diff file itself.
    }
    assert.ok(!("items" in file), "diff files do not carry a top-level items array");
  }
  assert.deepEqual([...statuses].sort(), ["added", "deleted", "modified"]);
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
});

test("codect diff with --path filter", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "signatures", "--path", "src/c.rs", "HEAD~1", "HEAD"],
    fixture
  );
  assert.equal(doc.schema, "codect.diff.v1");
  assert.equal(doc.files.length, 1);
  assert.equal(doc.files[0].path, "src/c.rs");
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

test("codect diff HEAD :worktree includes modified and untracked files", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "types", "HEAD", ":worktree"],
    snapshotFixture
  );

  assert.equal(doc.schema, "codect.diff.v1");
  assert.equal(doc.base.kind, "commit");
  assert.equal(doc.target.kind, "worktree");
  assert.equal(doc.target.revision, ":worktree");
  assert.ok(typeof doc.target.id === "string" && doc.target.id.length > 0);

  const byPath = new Map(doc.files.map((f) => [f.path, f]));
  assert.equal(byPath.get("src/tracked.rs")?.status, "modified");
  assert.equal(byPath.get("src/untracked.rs")?.status, "added");
  // `:worktree` honors the ignore stack: an ignored untracked file never leaks in.
  assert.equal(byPath.has("src/ignored.rs"), false);
});

test("codect diff :index :worktree isolates unstaged changes", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "types", ":index", ":worktree"],
    snapshotFixture
  );

  assert.equal(doc.base.kind, "index");
  assert.equal(doc.target.kind, "worktree");

  const tracked = doc.files.find((f) => f.path === "src/tracked.rs");
  assert.equal(tracked?.status, "modified");
  // The staged side carries the first edit, the worktree side the second.
  assert.ok(tracked.base.projection.text.includes("i64"));
  assert.ok(tracked.target.projection.text.includes("u64"));
});

test("codect diff :empty :worktree marks every file added", () => {
  const doc = runCodect(
    ["diff", "--format", "json", "--mode", "types", ":empty", ":worktree"],
    snapshotFixture
  );

  assert.equal(doc.base.kind, "empty");
  assert.equal(doc.target.kind, "worktree");
  assert.ok(doc.files.length > 0);
  assert.ok(doc.files.every((f) => f.status === "added"));
  assert.ok(doc.files.some((f) => f.path === "src/untracked.rs"));
});
