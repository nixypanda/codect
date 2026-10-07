/**
 * definition.ts — Sidebar tab definitions for the codect DSH plugin.
 *
 * Registers two tab kinds: "codect-show" and "codect-diff". Each appears
 * as its own tab type in the right sidebar.
 *
 * Pattern follows dsh-client-ui-sidebar-files/lib/types/client/definition.js.
 */

/** Tab kinds this plugin owns. */
export const SHOW_KIND = "codect-show";
export const DIFF_KIND = "codect-diff";

/** This plugin's identity in the tab system — matches package name. */
export const CODECT_ID = "@nixypanda/dsh-codect";

export type CodectTabKind = typeof SHOW_KIND | typeof DIFF_KIND;

export interface CodectTabDefinition {
  id: string;
  kind: CodectTabKind;
  priority: string;
  title: () => string;
  guide: CodectGuide[];
}

export interface CodectGuide {
  id: string;
  commandId: string;
  order: number;
  title: () => string;
  description: () => string;
  /** Guide icon name, or `null` to use the tab's default. */
  icon: string | null;
}

/** Definition for the "show" tab type. */
export function showDefinition(t: (key: string) => string): CodectTabDefinition {
  return {
    id: `${CODECT_ID}-show`,
    kind: SHOW_KIND,
    priority: "third-party",
    title: () => t("codect.title"),
    guide: [
      {
        id: "codect-show",
        commandId: "codect.show",
        order: 10,
        title: () => t("codect.guide.show.title"),
        description: () => t("codect.guide.show.description"),
        icon: null,
      },
    ],
  };
}

/** Definition for the "diff" tab type. */
export function diffDefinition(t: (key: string) => string): CodectTabDefinition {
  return {
    id: `${CODECT_ID}-diff`,
    kind: DIFF_KIND,
    priority: "third-party",
    title: () => t("codect.diff.title"),
    guide: [
      {
        id: "codect-diff",
        commandId: "codect.diff",
        order: 20,
        title: () => t("codect.guide.diff.title"),
        description: () => t("codect.guide.diff.description"),
        icon: null,
      },
    ],
  };
}
