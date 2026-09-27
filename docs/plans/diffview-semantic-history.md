# Plan: OwnAI projections in Diffview

## Goal

Keep the existing Diffview commands, panels, commit navigation, layouts, and
keymaps. In an OwnAI mode, each diff pane compares the canonical Types or
Signatures projection of its underlying Git snapshot. A file appears in a
focused file list only when those projections differ. Body-only edits therefore
produce no focused file diff. Users can switch the current view between
`types`, `signatures`, and ordinary source diffs without opening another UI.

This is a content integration, not the semantic folding proposed in the earlier
Neovim plan. Folds over real source are useful for `:OwnaiShow`, but Diffview
must receive projected text to show *only* projected changes.

## Commands to preserve

The current mappings in
[the dotfiles Diffview config](https://github.com/nixypanda/dotfiles/blob/master/modules/nvim/lua/diffview.lua)
are the acceptance surface:

| Key | Existing command | Focused-mode requirement |
| --- | --- | --- |
| `<leader>go` | `:DiffviewOpen` | Show projected staged and unstaged changes, keeping Diffview's two sections and file navigation. |
| `<leader>gc` | `:DiffviewClose` | Close the focused view and dispose of temporary projection buffers. |
| `<leader>gf` | `:DiffviewFileHistory` | Keep the commit list and commit selection; show the selected commit's projected file differences. |
| `<leader>gD` | `require("diffview").open({ base .. "...HEAD" })` | Compare the merge base and `HEAD` projections, preserving the existing nearest-ancestor branch choice. |
| `<leader>gL` | `:DiffviewFileHistory --range=base..HEAD` | Browse those commits in order, with a focused diff for each selected commit. |

The regular `:DiffviewOpen` and `:DiffviewFileHistory` commands with explicit
revisions, ranges, and paths should use the same integration. Keep the mapping
code's Git range semantics: `base...HEAD` for the aggregate branch comparison
and `base..HEAD` for the history's commit set.

## Behavior to define and test

- Mode is view-scoped. Opening any of the commands above while OwnAI focus is
  enabled uses the selected mode. Switching `types`/`signatures`/`source`
  refreshes the current Diffview tab, preserving its selected commit, file, and
  panel position when that selection still exists. Provide an OwnAI command or
  mapping for this switch; do not replace the existing five mappings.
- A history entry represents the same commit that Diffview selected. Its file
  comparisons use Diffview's chosen parent/base for that entry, including its
  merge-commit policy. A commit with no projected changes remains visible in
  the history with a clear "no Types/Signatures changes" state. Hiding such
  commits can be a later option; it must not make commit navigation jump
  unexpectedly in the first version.
- Files with equal projections are absent from the focused file list. Added
  and deleted files use an empty projection on the missing side. A supported
  file with no declarations also has an empty projection. Unsupported files
  are omitted from focused lists. Projection errors are reported explicitly;
  they must not silently turn into an empty diff.
- Neovim's built-in diff alignment, highlights, and `[c`/`]c` compare the
  *projected* buffer lines. Source mode restores the original Diffview data,
  including ordinary Git hunks. Index and worktree buffers retain their normal
  edit and stage behavior in source mode. Focused buffers are read-only.
- A file rename is shown as a deletion/addition if its path-dependent stable
  keys or projections require it; rename-aware semantic matching is separate
  work. Binary files have no focused projection.

## Architecture

1. **Projection data contract.** Add `ownai diff --format json` with a versioned
   `ownai.diff.v1` schema. Each changed file carries path/status, base and
   target snapshot identity, each side's projection text and items, and a
   semantic equality result. The first integration needs canonical text for
   Diffview; item spans and stable keys can support later declaration navigation
   and source drill-down. Keep `ownai.show.v1` and text `ownai diff` unchanged.
   Commit the schema, examples, and CLI contract documentation. The initial
   contract accepts two commits; index and worktree inputs are separate work.
2. **Snapshot inputs.** Model `commit`, `index`, `worktree`, and `absent` as
   distinct read-only inputs. Reuse the existing engine projection code and
   blob cache for commits. Add index access for staged content. Feed exact
   worktree buffer bytes to OwnAI when Diffview is showing an open, modified
   buffer; otherwise read the worktree file safely. Do not project `HEAD` as a
   stand-in for the index or a modified buffer. The CLI needs either a batch
   snapshot API or a narrow editor-facing request that accepts the exact two
   side contents plus their paths. Define the request before implementing the
   Lua bridge, so large history views do not require a process per file.
3. **Diffview integration seam.** Confirm the installed Diffview version and
   build a small proof that substitutes projected contents and the filtered
   file list for both `DiffviewOpen` and `DiffviewFileHistory`. Diffview's
   documented `diff_buf_read`/`diff_buf_win_enter` hooks run after buffers are
   created; they suffice for decoration but not for a trustworthy semantic
   file list or content source. Prefer a small, explicit content/list provider
   extension in Diffview upstream. If upstream cannot accept one, keep a
   version-pinned adapter or fork with the same provider seam. Do not rewrite
   or rename Diffview's worktree/index buffers in a hook: that would affect
   editing, staging, buffer reuse, and source-mode restoration.
4. **OwnAI provider.** Implement `editors/nvim/lua/ownai/diffview.lua` as an
   optional bridge. Given Diffview's exact comparison and selected mode, it
   requests projections, returns only changed supported files, and supplies
   read-only projected lines for both panes. Cache by repository, snapshot
   identity (commit ID, index state, or worktree content hash), path, and mode.
   Invalidate worktree/index entries on edits, writes, staging, and Diffview
   refresh; reuse immutable commit results while browsing history. Discard
   stale asynchronous responses when the user moves to another commit or mode.
5. **Configuration.** Expose `require("ownai").setup({ diffview = { ... } })` or
   an equivalent opt-in setup. Load the bridge after Diffview in the dotfiles
   `after` callback, and add one mode-switch binding if desired. Existing
   `:DiffviewOpen`, `:DiffviewFileHistory`, `:DiffviewClose`, and the nearest
   ancestor helpers continue to be called exactly as they are now. Package
   Diffview as an optional dependency of `ownai.nvim`; show and auto-fold work
   when it is absent.

## Delivery order

1. **Compatibility spike:** pin the Diffview revision from the actual Neovim
   setup; trace normal diff, file history, staged, and worktree buffer creation.
   Demonstrate a provider-fed two-file history entry and filtering of one
   body-only file. Decide whether the seam is upstreamable or needs an adapter.
2. **Backend:** implement the versioned diff document and snapshot inputs;
   validate schemas and all four languages against committed, index, and
   worktree fixtures. Preserve the existing focused-diff rule that equal
   projections are omitted.
3. **History first:** integrate `:DiffviewFileHistory` and the branch-range
   history mapping. This is the primary requested flow. Validate commit
   navigation, empty semantic commits, added/deleted files, and source-mode
   switching.
4. **Other Diffview commands:** enable `:DiffviewOpen`, its ancestor-branch
   wrapper, and close/refresh behavior with the same provider. Verify staged
   and unstaged sections separately.
5. **Polish and release:** document setup in `editors/nvim/README.md`, add
   headless integration tests against a pinned Diffview checkout, and add an
   optional Nix check. Test a real repository through the five dotfiles
   mappings before treating the feature as complete.

## Acceptance checks

- Selecting consecutive commits in `<leader>gf` or `<leader>gL` updates the
  projected panes and file list to that exact commit comparison.
- A body-only commit shows an empty focused state; a signature edit appears in
  Signatures and not Types; a type edit appears in both.
- `<leader>go` distinguishes `HEAD` vs index from index vs worktree, including
  unsaved worktree buffer content where Diffview displays it.
- `<leader>gD` compares the same merge-base range as the existing mapping.
- Changing mode cannot leave stale panes or file entries from a previous
  commit; source mode returns to unmodified Diffview behavior.
- `<leader>gc` closes the view cleanly. Ordinary `:OwnaiShow` still works with
  no Diffview installation.

## Dependencies and limits

The main dependency is a stable way to let Diffview obtain projected buffer
contents *and* a semantic file list, including within file history. Its public
hooks do not promise this. The compatibility spike should resolve that before
the backend and plugin grow around private Diffview objects. The initial scope
is Elm, Haskell, Python, and Rust, matching OwnAI's current projectors.
