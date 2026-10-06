/**
 * client.ts — Frontend client for the codect DSH plugin.
 *
 * Runs in the renderer process (web GUI). Registers the codect
 * sidebar tabs (show + diff) with the DSH sidebar system.
 *
 * Pattern follows dsh-client-ui-sidebar-files/lib/client.js:
 *   window.__ModuleLoader__.load({ id, factory(require) { ... } })
 *   No top-level imports — uses require() inside the factory.
 */

/* eslint-disable @typescript-eslint/no-unused-vars */
declare function require(id: string): any;

const React = require("react");
const { CODECT_ID, SHOW_KIND, DIFF_KIND, showDefinition, diffDefinition } = require("./definition");
const { ShowView } = require("./show-view");
const { DiffView } = require("./diff-view");
const { CodectTitle } = require("./title");
const { codeLabels } = require("./labels");
const { ensureStyles } = require("./styles");
const { fileAddressFor } = require("./address");
const { TYPERT_REMOTE } = require("../api-codect/typert.remote-client");

// ---------------------------------------------------------------------------
// Locale namespace
// ---------------------------------------------------------------------------

const NS = "codect";

const LOCALE_EN: Record<string, string> = {
  "codect.title": "Codect",
  "codect.diff.title": "Diff",
  "codect.guide.show.title": "Show Projection",
  "codect.guide.show.description": "Browse types and signatures for a revision",
  "codect.guide.diff.title": "Focused Diff",
  "codect.guide.diff.description": "Compare declarations between two revisions",
  "codect.label.revision": "Revision",
  "codect.label.mode": "Projection mode",
  "codect.label.pathFilter": "Filter by path",
  "codect.label.base": "Base revision",
  "codect.label.target": "Target revision",
  "codect.label.files": "files changed",
  "codect.label.declarations": "declarations",
  "codect.label.outline": "Outline",
  "codect.label.toggleTree": "Toggle file tree",
  "codect.label.diff": "Diff",
  "codect.label.diffView": "Diff view",
  "codect.label.diffLayout": "Diff layout",
  "codect.label.unified": "Unified",
  "codect.label.sideBySide": "Side by side",
  "codect.label.snapshots": "Snapshots",
  "codect.preset.worktree": "Worktree",
  "codect.preset.index": "Index",
  "codect.preset.staged": "Staged",
  "codect.preset.empty": "All files",
  "codect.placeholder.revision": "Revision (HEAD)",
  "codect.placeholder.base": "Base (HEAD~1)",
  "codect.placeholder.target": "Target (HEAD)",
  "codect.placeholder.path": "Filter by path",
  "codect.loading.show": "Loading projection…",
  "codect.loading.diff": "Loading focused diff…",
  "codect.empty.show": "No projected file content in this scope.",
  "codect.empty.diff": "No focused changes. Body-only edits are omitted.",
  "codect.label.code": "Code",
  "codect.label.copy": "Copy",
  "codect.label.copied": "Copied",
  "codect.label.wrap": "Wrap lines",
  "codect.label.unwrap": "Unwrap lines",
  "codect.label.window": "{shown} / {total} lines",
  "codect.label.expand": "Show {hidden} more lines",
  "codect.label.collapse": "Collapse",
  "codect.label.expandAria": "Show {hidden} more lines",
  "codect.label.collapseAria": "Collapse",
  "codect.shortcut.noSession": "No active session",
};

const LOCALE_ZH: Record<string, string> = {
  "codect.title": "Codect",
  "codect.diff.title": "差异",
  "codect.guide.show.title": "显示投影",
  "codect.guide.show.description": "浏览一个修订的类型和签名",
  "codect.guide.diff.title": "聚焦差异",
  "codect.guide.diff.description": "比较两个修订之间的声明",
  "codect.label.revision": "修订",
  "codect.label.mode": "投影模式",
  "codect.label.pathFilter": "按路径过滤",
  "codect.label.base": "基准修订",
  "codect.label.target": "目标修订",
  "codect.label.files": "个文件变更",
  "codect.label.declarations": "个声明",
  "codect.label.outline": "大纲",
  "codect.label.toggleTree": "切换文件树",
  "codect.label.diff": "差异",
  "codect.label.diffView": "差异视图",
  "codect.label.diffLayout": "差异布局",
  "codect.label.unified": "统一",
  "codect.label.sideBySide": "并排",
  "codect.label.snapshots": "快照",
  "codect.preset.worktree": "工作区",
  "codect.preset.index": "索引",
  "codect.preset.staged": "已暂存",
  "codect.preset.empty": "全部文件",
  "codect.placeholder.revision": "修订 (HEAD)",
  "codect.placeholder.base": "基准 (HEAD~1)",
  "codect.placeholder.target": "目标 (HEAD)",
  "codect.placeholder.path": "按路径过滤",
  "codect.loading.show": "正在加载投影…",
  "codect.loading.diff": "正在加载聚焦差异…",
  "codect.empty.show": "此范围内没有投影文件内容。",
  "codect.empty.diff": "没有聚焦变更。仅函数体的修改会被省略。",
  "codect.label.code": "代码",
  "codect.label.copy": "复制",
  "codect.label.copied": "已复制",
  "codect.label.wrap": "自动换行",
  "codect.label.unwrap": "取消换行",
  "codect.label.window": "{shown} / {total} 行",
  "codect.label.expand": "展开另外 {hidden} 行",
  "codect.label.collapse": "折叠",
  "codect.label.expandAria": "展开另外 {hidden} 行",
  "codect.label.collapseAria": "折叠",
  "codect.shortcut.noSession": "没有活动会话",
};

// ---------------------------------------------------------------------------
// Codect face (Remote client)
// ---------------------------------------------------------------------------

/**
 * Wrap the `remote.codect` namespace service in the small face the views use.
 * The caller must pass the namespace obtained from a context that injected
 * `remote.codect`; reading it off the raw `remote` service throws.
 */
function codectFace(codect: any) {
  return {
    async show(params: any) {
      const result = await codect.show(params);
      if (result && result.ok === false) throw result.error;
      return result && "value" in result ? result.value : result;
    },
    async diff(params: any) {
      const result = await codect.diff(params);
      if (result && result.ok === false) throw result.error;
      return result && "value" in result ? result.value : result;
    },
  };
}

// ---------------------------------------------------------------------------
// Optional client services
// ---------------------------------------------------------------------------

/**
 * Read a client service without declaring an injection requirement.
 *
 * Returns `undefined` when the service is absent or another plugin's fiber
 * provided it but is not active. Used for soft dependencies such as the
 * community `dsh-diff-view` client service (`diffView`), which must degrade to
 * the unified `DiffBlock` rather than fail activation.
 */
function optionalService(ctx: any, name: string): any {
  try {
    return typeof ctx?.get === "function" ? ctx.get(name, false) : undefined;
  } catch {
    return undefined;
  }
}

// ---------------------------------------------------------------------------
// Apply — register everything
// ---------------------------------------------------------------------------

/**
 * Services the client context must carry before `apply` runs. The codect
 * namespace itself is mounted by `apply` (see TYPERT_REMOTE below), so only the
 * `remote` service it mounts onto is required here.
 */
const inject = [
  "slots",
  "locale",
  "sidebarRightTabs",
  "sidebarRight",
  "remote",
];

/**
 * The sidebar tab body receives a session-scoped props bag from the sidebar
 * service (see dsh-client-ui-sidebar-files). Derive the workspace root from the
 * session's cwd, falling back to "." when it is not yet known.
 */
function useRoot(props: any): string {
  const cwd =
    props && typeof props.useSessions === "function" && props.sessionId !== undefined
      ? props.useSessions((sessions: any) => sessions?.byId?.[props.sessionId]?.cwd)
      : undefined;
  return cwd || props?.root || ".";
}

/**
 * Register the two tab types, their bodies and titles, the dictionaries, and
 * the shortcuts that open each tab in the right sidebar.
 *
 * This is the ModuleLoader `apply` entry: it is called with the client root
 * context once the injected services are present.
 */
function apply(ctx: any) {
  const t = (key: string) => LOCALE_EN[key] || key;
  ensureStyles();
  const labels = codeLabels(t);

  // Side-by-side rendering is provided by the optional community
  // `dsh-diff-view` plugin through its `diffView` client service. It is not in
  // the platform module table, so read it leniently and let `inject` refresh the
  // holder if the provider loads after this plugin.
  const diffView: { current: any } = { current: optionalService(ctx, "diffView") };
  ctx.inject(["diffView"], (scoped: any) => {
    diffView.current = scoped.diffView;
    return () => {
      diffView.current = null;
    };
  });

  // Mount the Client FaceModel contribution to install the `remote.codect`
  // namespace before anything injects it.
  if (ctx.remote && typeof ctx.remote.$mount === "function") {
    ctx.effect(
      () => ctx.remote.$mount(TYPERT_REMOTE),
      "codect: remote namespace"
    );
  }

  function openTab(kind: string) {
    return ({ target: element }: { target: any }) => {
      const target = ctx.sidebarRight.commandTarget(element);
      if (target === undefined) {
        return { status: "blocked", reason: t("codect.shortcut.noSession") };
      }
      return {
        status: "handled",
        run: () => {
          ctx.sidebarRight.openTabFromTarget(kind, target);
        },
      };
    };
  }

  // Register shortcuts that open each tab from the keyboard.
  ctx.inject(["shortcuts"], (shortcutsCtx: any) => {
    shortcutsCtx.register({
      id: "codect.show",
      label: () => t("codect.guide.show.title"),
      aliases: ["codect show", "codect"],
      defaults: {
        "desktop:macos": { code: "KeyJ", modifiers: ["primary", "shift"] },
        "desktop:windows": { code: "KeyJ", modifiers: ["ctrl", "shift"] },
        "web:macos": { code: "KeyJ", modifiers: ["primary", "shift"] },
        "web:windows": { code: "KeyJ", modifiers: ["ctrl", "shift"] },
      },
      regions: ["page", "editable", "terminal"],
      modals: [],
      resolve: openTab(SHOW_KIND),
    });

    shortcutsCtx.register({
      id: "codect.diff",
      label: () => t("codect.guide.diff.title"),
      aliases: ["codect diff"],
      defaults: {
        "desktop:macos": { code: "KeyK", modifiers: ["primary", "shift"] },
        "desktop:windows": { code: "KeyK", modifiers: ["ctrl", "shift"] },
        "web:macos": { code: "KeyK", modifiers: ["primary", "shift"] },
        "web:windows": { code: "KeyK", modifiers: ["ctrl", "shift"] },
      },
      regions: ["page", "editable", "terminal"],
      modals: [],
      resolve: openTab(DIFF_KIND),
    });
  });

  // Register tab types.
  ctx.effect(() => {
    ctx.sidebarRightTabs.register(showDefinition(t));
  }, "codect: show type");

  ctx.effect(() => {
    ctx.sidebarRightTabs.register(diffDefinition(t));
  }, "codect: diff type");

  // Register dictionaries.
  ctx.effect(() => {
    ctx.locale.register(NS, { en: LOCALE_EN, zh: LOCALE_ZH });
  }, "codect: dictionaries");

  // The bodies call the Remote namespace, so obtain it through an inject scope:
  // reading `remote.codect` from a context that did not inject it throws.
  ctx.inject(["remote.codect"], (scoped: any) => {
    const codect = codectFace(scoped.remote.codect);

    /**
     * Build the navigation callback for a sidebar body. It opens the real
     * source file at the declaration line, in the tab's own pane when the
     * framework exposes `useTabInfo().tab.actions`, else in the active pane.
     */
    function navigateFrom(
      props: any,
      tabActions: any,
      root: string
    ): (path: string, line: number) => void {
      return (path: string, line: number) => {
        const sessionId = props?.sessionId;
        if (sessionId === undefined || sessionId === null) return;
        const address = fileAddressFor(String(sessionId), root, path);
        if (tabActions && typeof tabActions.openResource === "function") {
          tabActions.openResource(address, { params: { line } });
          return;
        }
        if (ctx.sidebarRight && typeof ctx.sidebarRight.openResource === "function") {
          ctx.sidebarRight.openResource(address, { params: { line } });
        }
      };
    }

    function ShowBody(props: any) {
      const root = useRoot(props);
      const tabInfo = typeof props?.useTabInfo === "function" ? props.useTabInfo() : undefined;
      const onNavigate = navigateFrom(props, tabInfo?.tab?.actions, root);
      return React.createElement(ShowView, { root, codect, labels, t, onNavigate });
    }

    function DiffBody(props: any) {
      const root = useRoot(props);
      const tabInfo = typeof props?.useTabInfo === "function" ? props.useTabInfo() : undefined;
      const onNavigate = navigateFrom(props, tabInfo?.tab?.actions, root);
      return React.createElement(DiffView, {
        root,
        codect,
        labels,
        t,
        onNavigate,
        diffView: diffView.current,
      });
    }

    ctx.effect(() => {
      ctx.slots.inject("sidebar.right.pane.tab", () => {
        ctx.slots.register(
          {
            name: "sidebar.right.pane.tab",
            key: `${CODECT_ID}-show`,
            locale: NS,
            children: {},
          },
          ShowBody
        );
      });
    }, "codect: show tab body");

    ctx.effect(() => {
      ctx.slots.inject("sidebar.right.pane.tab", () => {
        ctx.slots.register(
          {
            name: "sidebar.right.pane.tab",
            key: `${CODECT_ID}-diff`,
            locale: NS,
            children: {},
          },
          DiffBody
        );
      });
    }, "codect: diff tab body");
  });

  // Register tab titles.
  ctx.effect(() => {
    ctx.slots.inject("sidebar.right.pane.tab.title", () => {
      ctx.slots.register(
        {
          name: "sidebar.right.pane.tab.title",
          key: `${CODECT_ID}-show`,
        },
        () => React.createElement(CodectTitle, { kind: SHOW_KIND })
      );
    });
  }, "codect: show tab title");

  ctx.effect(() => {
    ctx.slots.inject("sidebar.right.pane.tab.title", () => {
      ctx.slots.register(
        {
          name: "sidebar.right.pane.tab.title",
          key: `${CODECT_ID}-diff`,
        },
        () => React.createElement(CodectTitle, { kind: DIFF_KIND })
      );
    });
  }, "codect: diff tab title");
}

// ModuleLoader registration is added by the build script (scripts/build.mjs)
// which wraps this file in window.__ModuleLoader__.load({ id, factory }).
// The named exports become the properties of the factory's module.exports.
export { apply, inject, codectFace, useRoot };
