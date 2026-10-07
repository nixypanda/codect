# DeepSeek Harness (DSH) sidebar plugin

Codect ships one integration per editor host. This note records how the DSH
sidebar plugin fits the DeepSeek Harness web client, and the platform facts and
lessons behind its design. For setup, the module map, and how to build it, see
[`editors/dsh/README.md`](../../editors/dsh/README.md).

## Where the plugin lives

```
editors/dsh/
├── package.json                 # @nixypanda/dsh-codect, dsh.bundle.patch, dsh.client
├── cordis.patch.yml             # inserts [{ id: codect, name: @nixypanda/dsh-codect }]
├── scripts/build.mjs            # esbuild: lib/index.js (host), lib/client.js (client)
├── src/api-codect/              # host service (TypertRemoteService, @Remote show/diff)
├── src/client-codect/           # client (ModuleLoader entry, views, tree, styles)
├── src/shared/schema.ts         # codect.show.v1 / codect.diff.v1 types
└── test/                        # CLI JSON contract + pure-helper unit tests
```

## What DSH exposes to plugins

Read from the installed app at DSH `0.2.0-rc.2` (paths are inside `app.asar`).
These are the seams a plugin may use.

### Static platform module table

From `dsh/node_modules/@deepseek-ai/dsh-web-frontend/dist/assets/index-*.js`:

```
react, react/jsx-runtime, react-dom, react-dom/client,
@deepseek-ai/cordis, @deepseek-ai/dsh-client-store,
@deepseek-ai/dsh-client-ui-slots, @deepseek-ai/dsh-client-ui-primitives,
@deepseek-ai/dsh-client-ui-dockkit
```

A plugin may `require` these and must declare every non-baseline one in
`dsh.client.external` (React/Cordis are baseline). The loader refuses undeclared
non-baseline requests. `@deepseek-ai/dsh-client-ui-primitives` is the sanctioned
shared control library: *"A plugin cannot import another plugin's component, so
this package is the only place a control can be shared."* Its catalog includes
`Button`, `Menu`, `MenuItemButton`, `Input`, `SegmentedControl`, `SegmentedTabs`,
`Pill`, `Tag`, `PathLabel`, `StateDot`, `DisclosureRow`, `TextShimmer`, and
`ConnectionIndicator`.

### Tab types over `dsh-resource://` addresses

From `@deepseek-ai/dsh-client-ui-sidebar-right`:

```ts
ctx.sidebarRightTabs.register({
  id,            // unique implementation identity (a package name)
  kind,          // page kind; the body registers under this key
  patterns,      // globs over dsh-resource:// addresses
  priority,      // band: extension | builtin | fallback
  canOpen,       // optional predicate
  title, guide, keepMounted,
}); // returns a disposer
ctx.sidebarRight.openResource(address, options?);
ctx.sidebarRight.openTab(kind, options?);
```

A pattern containing `:` matches the whole address (`dsh-resource://file/**`);
one without matches the URI path at any depth (`*.md`). The
`sidebar.right.pane.tab` seat renders the body; the
`sidebar.right.pane.tab.title` seat renders the chip. This gives tab identity,
layout persistence, the guide, and the add-tab flow.

### Custom resource protocols

From `@deepseek-ai/dsh-client-resources`:

```ts
export const inject = ["resources"];
ctx.resources.register({
  protocol: "codect",
  async *open(address, { signal }) {
    yield okFrame;           // { ok: true, value }
    for await (const change of follow) yield change;
    // must stop on signal abort
  },
});
// slot components read useResource("dsh-resource://codect/...").
```

`ctx.resources.pin(address, signal)` holds a resource open; the sidebar pins
every open tab. `ctx.resources.source(address)` is the observable.

### Document previews

`@deepseek-ai/dsh-client-ui-sidebar-documentpreview` registers a tab type for
`dsh-resource://file/**` and a body that chooses among renderers registered with
`ctx.documentPreviews.register({ id, extensions, binaryExtensions?, priority,
title, loading, wrap? })`, with a body under the `sidebar.right.tab.document`
child slot. It is file-address oriented; it is the reference for a code renderer
(paged text, wrap, reload, syntax colours) but not directly reusable for
arbitrary resources.

### What is **not** exposed

- **No plugin-facing diff component.** The only shipped diff UI is the
  `changes-review` tab inside `@deepseek-ai/dsh-client-ui-deliverables`
  (unified/side-by-side toggle, hunk headers, old/new line numbers, Shiki token
  colours). It is hard-wired to the Host `workspace/changes` routes and to its
  own address `dsh-resource://changes-review/session/<id>/<seq>/<turn>`; it is
  not a service, slot, or exported component. The authoring rules forbid
  importing another plugin's component.
- **No reusable file-tree component.** `@deepseek-ai/dsh-client-ui-sidebar-files`
  renders a tree over `@deepseek-ai/dsh-api-workspace-files`; it is a package,
  not an API. Its tree and its register-spec store/inject pattern are a
  reference to copy, not import.

### Why Neovim can piggyback and DSH cannot

`codect.nvim` integrates with **Diffview**, which publishes a provider/adapter
contract (file lists, pane contents), so Codect implements an adapter and guards
it by source hash. DSH has no equivalent diff/tree provider contract — only tab
types, resource protocols, and shared primitives. Piggybacking in DSH means
either copying the `changes-review` rendering or consuming a service a community
plugin publishes.

## Diff rendering decision

Codect consumes the community `dsh-diff-view` plugin, which publishes a
`diffView` client service rendering base/target text with Shiki highlighting,
intra-line marks, real line numbers, and context collapse. The plugin treats it
as an optional service (`optionalService` + `ctx.inject`, so provider load order
does not matter) and falls back to the platform `DiffBlock` and then a
plain-text dump. This copies no alignment engine into Codect and needs no
upstream DSH change. The alternatives — duplicating the `changes-review`
rendering, or adding a generic diff capability to DSH — were rejected and the
decision recorded here.

## Host ↔ client wiring (do not regress)

- The host service must extend `TypertRemoteService` (or bind via
  `bindTypertRemote`) for `@Remote` to register.
- Pass the host `Config` schema to the constructor (`constructor(ctx, config)`);
  `@deepseek-ai/schemastery` is a default import (`import z from …`).
- The Typert manifest must satisfy `dsh-typert-loader` validation (`tags`,
  `exportName`, service `members`/`types`).
- Install the package as an npm **tarball**, not a `link:` path, or pnpm skips
  its dependencies and the host import fails.
- The client mounts `TYPERT_REMOTE` and obtains the namespace through an
  `inject` scope; reading `remote.codect` off the raw `remote` service throws
  `cannot get property "remote.codect" without inject`.
- Remote calls resolve to an envelope `{ ok: true, value }` or
  `{ ok: false, error }`, not the raw document. Unwrap in the client face.
- The client bundle is built by `scripts/build.mjs` as CJS and wrapped in
  `window.__ModuleLoader__.load({ id: "@nixypanda/dsh-codect", factory })`.
  React comes from the platform table; do not bundle `zod` into the client.
- `codect.diff.v1` files carry `base`/`target` sides, not a per-file `items`
  array.

## References

- Plugin: [`editors/dsh/README.md`](../../editors/dsh/README.md).
- Schemas: [`docs/schema/codect.show.v1.json`](../schema/codect.show.v1.json),
  [`docs/schema/codect.diff.v1.json`](../schema/codect.diff.v1.json),
  [`docs/agents/CONTRACT.md`](../agents/CONTRACT.md).
- DSH packages (inside `app.asar/dsh/node_modules/@deepseek-ai/`):
  `dsh-client-ui-sidebar-right` (tab types, `openResource`),
  `dsh-client-resources` (resource protocols),
  `dsh-client-ui-sidebar-documentpreview` (code/preview renderers),
  `dsh-client-ui-sidebar-files` (tree reference),
  `dsh-client-ui-deliverables` (`changes-review` diff renderer reference),
  `dsh-client-ui-primitives` (shared controls),
  `dsh-client-modules` (ModuleLoader, `dsh.client.external`, platform table).
- Neovim for contrast: [`editors/nvim/README.md`](../../editors/nvim/README.md).
