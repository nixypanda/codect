# Adversarial review — PR #23: DeepSeek Harness (DSH) sidebar plugin

- **PR:** <https://github.com/nixypanda/codect/pull/23>
- **Branch:** `feat/dsh-editor-plugin`
- **Reviewed head:** `8b809c4` (`refs/pull/23/head`)
- **Date:** 2026-10-06
- **Method:** read every source file, the flake, the docs and the tests; ran
  `npm test`, `npm run typecheck`, `npm run build`, and `nix build .#codect-dsh`.

## Verdict

The blocking findings, the High items, and the Medium items with behavioral
impact from the original review have been resolved, and DSH host and client
activation have been validated against the real runtime. The findings below are
the ones still open. IDs are retained from the original review so they stay
traceable.

---

## Medium

### M3. Tab title chip never shows the revision

`title.tsx` accepts `revision`/`base`/`target` and renders them, but
`client.ts:381,393` constructs it with only `{ kind }`. The chip is permanently
just "Codect" / "Diff" — dead feature, and the PR's claim that the chip "resolves
to a 4px gap" is about CSS, not content.

### M4. Diff tree collapse isn't persisted; Show's is

`show-view.tsx:137-145` persists folds via `writeCollapsed`;
`diff-view.tsx:237-244` toggles an in-memory `Set` only. Inconsistent with the
plan's "sticky expansion" goal and with the sibling view.

---

## Low / nits

- `client.ts:381,393`: `CodectTitle` rendered with no `keepMounted` on the tab
  definitions, so switching tabs resets tree/selection state.
- `address.ts:51-55`: an absolute path outside the workspace yields
  `…/session/<id>//elsewhere/…` (double slash). The unit test
  `fileAddressFor … /elsewhere/a.rs` **enshrines** this, so the test would have to
  change with a fix.
- `schema.ts:152-156`: `CODEXT_EXIT` is misspelled (`CODEXT`) and exported but
  unused.
- `diff-view.tsx:344`: `document.base.revision` for a `:empty` side shows
  `:empty`; fine, but the summary doesn't humanize the kinds.
- `ShowResult.input` in `typert.host.ts` includes `stdin` even though the client
  view never offers it — harmless but shows schema/code drift.

---

## Verified non-issues

- M6 (`optionalService` + `ctx.inject(["diffView"])`): cordis
  `ctx.inject(deps, cb)` is shorthand for `ctx.plugin({ inject, apply })`, and a
  service that never appears leaves the child fiber pending rather than failing
  the parent. `optionalService` already seeds `diffView = undefined`, so the
  plugin activates and degrades to `DiffBlock` as documented.

---

## What's actually good

- Pure `tree.ts` / `outline.ts` / `address.ts` / `declaration-diff.ts` are clean,
  dependency-free, and genuinely unit-tested (26 tests, all green with the right
  binary). Directory-first natural ordering and the cycle-guarded depth walk are
  correct.
- `execFile` (not a shell) and a bounded `maxBuffer` are the right call.
- The schema mirror in `shared/schema.ts` matches `codect.diff.v1`'s side-based
  shape; the earlier "per-file `items`" mistake is fixed.
- The ModuleLoader wrapper produced by `build.mjs` is structurally sound
  (verified: `require("react")`/`require("react/jsx-runtime")` and the primitives
  seam are preserved, `apply`/`inject` land on `module.exports`).
- `nix build .#codect-dsh` builds cleanly.
- Docs (`docs/editors/dsh.md`) capture real platform findings and the non-obvious
  wiring lessons; the `--dsw-*` token theming and the "degrade to plain markup"
  seams are thoughtful.

---

## Remaining work

1. Address M3 (title chip content) and M4 (persist diff-tree folds) if the
   polish is wanted.
2. Clear the low nits: `keepMounted` on tab definitions, the `//` address edge
   case, the `CODEXT_EXIT` typo, and summary humanization.
