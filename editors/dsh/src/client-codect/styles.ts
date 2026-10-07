/**
 * styles.ts — Runtime stylesheet for the codect sidebar views.
 *
 * The plugin's esbuild pipeline bundles only JS (no CSS loader), so the sheet
 * is injected once into the document head. Every colour and radius resolves
 * through the DSH `--dsw-*` theme tokens with a neutral fallback, so the views
 * follow both light and dark palettes.
 *
 * The layout is a container query on the view root: the file tree sits on the
 * left beside the content at wide widths and stacks above it in a narrow
 * sidebar, mirroring the TUI's 80-column split.
 */

export const STYLE_ID = "codect-dsh-styles";

const CSS = `
.codect-view {
  container-type: inline-size;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  font-family: var(--dsw-font-family, system-ui, sans-serif);
  font-size: 12px;
  color: var(--dsw-alias-label-primary, inherit);
}
.codect-controls {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  align-items: center;
  padding: 6px;
  border-bottom: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.22));
}
.codect-field { flex: 1 1 84px; min-width: 0; }
.codect-input {
  width: 100%;
  box-sizing: border-box;
  padding: 3px 6px;
  border: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.28));
  border-radius: var(--dsw-radius-sm, 4px);
  background: var(--dsw-alias-bg-layer-2, rgba(127, 127, 127, 0.06));
  color: inherit;
  font: inherit;
}
.codect-input::placeholder { color: var(--dsw-alias-label-tertiary, rgba(127, 127, 127, 0.8)); }
.codect-input:focus { outline: none; border-color: var(--dsw-focus-ring-color, currentColor); }
.codect-segmented { flex: none; }
.codect-snapshots {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  padding: 4px 6px;
  border-bottom: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.22));
}
.codect-snapshots-label {
  flex: none;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--dsw-alias-label-tertiary, inherit);
}
.codect-snapshot {
  flex: none;
  padding: 2px 8px;
  border: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.28));
  border-radius: 999px;
  background: transparent;
  color: inherit;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}
.codect-snapshot:hover { background: var(--dsw-alias-interactive-bg-hover, rgba(127, 127, 127, 0.1)); }
.codect-snapshot[aria-pressed="true"] {
  background: var(--dsw-alias-interactive-bg-active, rgba(127, 127, 127, 0.16));
  border-color: var(--dsw-focus-ring-color, currentColor);
}
.codect-tree-toggle {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.28));
  border-radius: var(--dsw-radius-sm, 4px);
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}
.codect-tree-toggle:hover { background: var(--dsw-alias-interactive-bg-hover, rgba(127, 127, 127, 0.1)); }
.codect-tree-toggle[aria-pressed="true"] {
  background: var(--dsw-alias-interactive-bg-active, rgba(127, 127, 127, 0.16));
}
.codect-main {
  display: flex;
  flex-direction: column;
  flex: 1 1 auto;
  min-height: 0;
}
.codect-tree-pane {
  flex: 0 0 auto;
  max-height: 38%;
  overflow: auto;
  padding: 8px 0 8px 8px;
  border-bottom: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.22));
}
.codect-content-pane {
  flex: 1 1 auto;
  min-height: 0;
  overflow: auto;
  padding: 4px;
}
.codect-splitter {
  display: none;
  flex: 0 0 5px;
  cursor: col-resize;
  background: var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.18));
}
.codect-splitter:hover,
.codect-splitter:active {
  background: var(--dsw-focus-ring-color, currentColor);
}
.codect-main[data-resizing="true"] { user-select: none; }
.codect-content { display: flex; flex-direction: column; gap: 6px; }
.codect-diff-surface {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}
@container (min-width: 560px) {
  .codect-main { flex-direction: row; }
  .codect-tree-pane {
    flex: 0 0 var(--codect-tree-width, 40%);
    width: var(--codect-tree-width, 40%);
    max-width: 480px;
    max-height: none;
    border-bottom: none;
    border-right: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.22));
  }
  .codect-splitter { display: block; }
}
.codect-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 10px;
  cursor: pointer;
  white-space: nowrap;
  line-height: 1.5;
  border-radius: var(--dsw-radius-md, 6px);
}
.codect-row:hover { background: var(--dsw-alias-interactive-bg-hover, rgba(127, 127, 127, 0.1)); }
.codect-row-selected { background: var(--dsw-alias-interactive-bg-active, rgba(127, 127, 127, 0.16)); }
.codect-twisty {
  width: 12px;
  flex: none;
  text-align: center;
  color: var(--dsw-alias-label-tertiary, inherit);
}
.codect-tree-icon {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
}
.codect-row-label { overflow: hidden; text-overflow: ellipsis; }
.codect-row-dir { color: var(--dsw-alias-label-primary, inherit); }
.codect-badge {
  margin-left: auto;
  flex: none;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.04em;
}
.codect-badge-added { color: var(--dsw-alias-code-diff-added, #3fb950); }
.codect-badge-deleted { color: var(--dsw-alias-state-error-primary, #f85149); }
.codect-badge-modified { color: var(--dsw-alias-state-warn-primary, #d29922); }
.codect-file-header {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 1px 2px;
  color: var(--dsw-alias-label-secondary, inherit);
  overflow: hidden;
}
.codect-file-path {
  font-family: var(--dsw-font-markdown-code-block, monospace);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.codect-file-meta { flex: none; color: var(--dsw-alias-label-tertiary, inherit); font-size: 11px; }
.codect-summary {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: 2px 6px;
  padding: 3px 6px;
  color: var(--dsw-alias-label-secondary, inherit);
  white-space: normal;
  overflow: visible;
  overflow-wrap: anywhere;
}
.codect-summary code { font-family: var(--dsw-font-markdown-code-block, monospace); }
.codect-title {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
}
.codect-title-icon { display: inline-flex; align-items: center; flex: none; }
.codect-title-text,
.codect-title-rev { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.codect-title-rev {
  color: var(--dsw-alias-label-tertiary, inherit);
  font-size: 11px;
}
.codect-section {
  border: 1px solid var(--dsw-alias-border-l3, rgba(127, 127, 127, 0.22));
  border-radius: var(--dsw-radius-sm, 4px);
  overflow: hidden;
}
.codect-section-header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 3px 6px;
  border: none;
  background: var(--dsw-alias-bg-layer-2, rgba(127, 127, 127, 0.05));
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
}
.codect-section-header:hover { background: var(--dsw-alias-interactive-bg-hover, rgba(127, 127, 127, 0.1)); }
.codect-section-chevron { width: 12px; flex: none; color: var(--dsw-alias-label-tertiary, inherit); }
.codect-section-title { font-weight: 600; }
.codect-section-meta { margin-left: auto; color: var(--dsw-alias-label-tertiary, inherit); font-size: 11px; }
.codect-section-body { padding: 2px 0; }
.codect-outline { display: flex; flex-direction: column; }
.codect-outline-row {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 1px 6px;
  cursor: pointer;
  border-radius: var(--dsw-radius-sm, 4px);
  white-space: nowrap;
  overflow: hidden;
}
.codect-outline-row:hover { background: var(--dsw-alias-interactive-bg-hover, rgba(127, 127, 127, 0.1)); }
.codect-outline-kind {
  flex: none;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  color: var(--dsw-alias-label-tertiary, inherit);
}
.codect-outline-name { flex: none; font-weight: 600; }
.codect-outline-sig {
  flex: 1 1 auto;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--dsw-alias-label-secondary, inherit);
  font-family: var(--dsw-font-markdown-code-block, monospace);
}
.codect-outline-dim { opacity: 0.55; }
.codect-empty,
.codect-loading { padding: 14px 8px; color: var(--dsw-alias-label-tertiary, inherit); }
.codect-error {
  margin: 6px;
  padding: 8px;
  border-radius: var(--dsw-radius-sm, 4px);
  background: var(--dsw-alias-state-error-secondary, rgba(248, 81, 73, 0.12));
  color: var(--dsw-alias-state-error-primary, #f85149);
  white-space: pre-wrap;
  word-break: break-word;
}
.codect-toolbar-row { display: flex; gap: 6px; align-items: center; padding: 0 2px; }
.codect-link {
  background: none;
  border: none;
  padding: 0;
  color: var(--dsw-alias-label-secondary, inherit);
  font: inherit;
  cursor: pointer;
  text-decoration: underline dotted;
}
.codect-link:hover { color: var(--dsw-alias-label-primary, inherit); }
.codect-fallback-code {
  margin: 0;
  padding: 6px;
  overflow: auto;
  font-family: var(--dsw-font-markdown-code-block, monospace);
  font-size: 12px;
  white-space: pre;
}
.codect-decl-marker {
  flex: none;
  width: 12px;
  text-align: center;
  font-weight: 700;
}
.codect-decl-added .codect-decl-marker { color: var(--dsw-alias-code-diff-added, #3fb950); }
.codect-decl-deleted .codect-decl-marker { color: var(--dsw-alias-state-error-primary, #f85149); }
.codect-decl-changed .codect-decl-marker { color: var(--dsw-alias-state-warn-primary, #d29922); }
`;

/** Inject the stylesheet once per document. Safe to call repeatedly. */
export function ensureStyles(doc?: Document | null): void {
  const target = doc ?? (typeof document === "undefined" ? null : document);
  if (target === null) return;
  if (target.getElementById(STYLE_ID) !== null) return;
  const style = target.createElement("style");
  style.id = STYLE_ID;
  style.textContent = CSS;
  target.head.appendChild(style);
}
