/**
 * labels.ts — Localized label bundles for the DSH output cards.
 *
 * `ReadBlock` and `DiffBlock` are zero-Cordis atoms: every user-facing string
 * arrives through their required `labels` prop. This builds that bundle from
 * the plugin's `t` function.
 */

/** The shared code-card label shape used by `ReadBlock` and `DiffBlock`. */
export interface CodeLabels {
  codeLabel: string;
  copy: string;
  copied: string;
  wrapLabel: string;
  unwrapLabel: string;
  window: (shown: number, total: number) => string;
  expand: (hidden: number) => string;
  collapse: string;
  expandAria: (hidden: number) => string;
  collapseAria: string;
}

export function codeLabels(t: (key: string) => string): CodeLabels {
  return {
    codeLabel: t("codect.label.code"),
    copy: t("codect.label.copy"),
    copied: t("codect.label.copied"),
    wrapLabel: t("codect.label.wrap"),
    unwrapLabel: t("codect.label.unwrap"),
    window: (shown, total) => t("codect.label.window").replace("{shown}", String(shown)).replace("{total}", String(total)),
    expand: (hidden) => t("codect.label.expand").replace("{hidden}", String(hidden)),
    collapse: t("codect.label.collapse"),
    expandAria: (hidden) => t("codect.label.expandAria").replace("{hidden}", String(hidden)),
    collapseAria: t("codect.label.collapseAria"),
  };
}
