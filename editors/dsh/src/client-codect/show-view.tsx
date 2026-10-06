/**
 * show-view.tsx — Codect "Show" view.
 *
 * A collapsible file tree on the left beside the canonical projection text of
 * the selected file, with a collapsible declaration-outline section above it.
 * Mirrors the TUI `show` page (`crates/tui/src/page/show.rs` +
 * `component/tree.rs`) as native DSH UI: side by side when the pane is wide
 * enough, stacked when it is not.
 */

import React, { useCallback, useEffect, useMemo, useState } from "react";
import type { CodectMode, ShowDocument, ShowFile } from "../shared/schema";
import type { CodeLabels } from "./labels";
import { buildTreeRows } from "./tree";
import { outlineRows } from "./outline";
import { CodePane } from "./components/code-pane";
import { OutlineView } from "./components/outline-view";
import { TreeView } from "./components/tree-view";
import { ModeToggle } from "./components/mode-toggle";
import { SplitPane } from "./components/split-pane";

export interface ShowViewProps {
  root: string;
  codect: {
    show: (params: {
      root: string;
      revision?: string;
      mode: CodectMode;
      paths?: string[];
      areas?: string[];
    }) => Promise<ShowDocument>;
  };
  labels: CodeLabels;
  t: (key: string) => string;
  onNavigate?: (path: string, line: number) => void;
}

const COLLAPSED_KEY = "codect.show.collapsed";
const TREE_COLLAPSED_KEY = "codect.show.treeCollapsed";
const OUTLINE_OPEN_KEY = "codect.show.outlineOpen";

function readBool(key: string, fallback: boolean): boolean {
  try {
    const raw = globalThis.localStorage?.getItem(key);
    return raw === null || raw === undefined ? fallback : raw === "true";
  } catch {
    return fallback;
  }
}

function writeBool(key: string, value: boolean): void {
  try {
    globalThis.localStorage?.setItem(key, String(value));
  } catch {
    // Persisting view state is best-effort.
  }
}

function readCollapsed(): Set<string> {
  try {
    const raw = globalThis.localStorage?.getItem(COLLAPSED_KEY);
    if (raw === null || raw === undefined) return new Set();
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? new Set(parsed) : new Set();
  } catch {
    return new Set();
  }
}

function writeCollapsed(collapsed: Set<string>): void {
  try {
    globalThis.localStorage?.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed]));
  } catch {
    // Persisting folds is best-effort.
  }
}

export function ShowView({ root, codect, labels, t, onNavigate }: ShowViewProps) {
  const [document, setDocument] = useState<ShowDocument | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [revision, setRevision] = useState("HEAD");
  const [mode, setMode] = useState<CodectMode>("types");
  const [pathFilter, setPathFilter] = useState("");
  const [collapsed, setCollapsed] = useState<Set<string>>(() => readCollapsed());
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [treeCollapsed, setTreeCollapsed] = useState<boolean>(() =>
    readBool(TREE_COLLAPSED_KEY, false)
  );
  const [outlineOpen, setOutlineOpen] = useState<boolean>(() =>
    readBool(OUTLINE_OPEN_KEY, false)
  );

  const loadProjection = useCallback(async () => {
    if (!root) return;
    setLoading(true);
    setError(null);
    try {
      const doc = await codect.show({
        root,
        revision: revision || undefined,
        mode,
        paths: pathFilter ? [pathFilter] : undefined,
      });
      setDocument(doc);
      setSelectedPath((current) => {
        if (current !== null && doc.files.some((file) => file.path === current)) {
          return current;
        }
        return doc.files.length > 0 ? doc.files[0].path : null;
      });
    } catch (e) {
      setError((e as Error).message);
      setDocument(null);
    } finally {
      setLoading(false);
    }
  }, [root, revision, mode, pathFilter, codect]);

  useEffect(() => {
    void loadProjection();
  }, [loadProjection]);

  const files = document?.files ?? [];
  const rows = useMemo(
    () => buildTreeRows(files.map((file) => file.path), collapsed),
    [files, collapsed]
  );
  const selectedFile: ShowFile | undefined = files.find(
    (file) => file.path === selectedPath
  );
  const outline = useMemo(
    () => outlineRows(selectedFile?.outline ?? []),
    [selectedFile]
  );

  const toggleDir = useCallback((path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      writeCollapsed(next);
      return next;
    });
  }, []);

  const toggleTree = useCallback(() => {
    setTreeCollapsed((prev) => {
      writeBool(TREE_COLLAPSED_KEY, !prev);
      return !prev;
    });
  }, []);

  const toggleOutline = useCallback(() => {
    setOutlineOpen((prev) => {
      writeBool(OUTLINE_OPEN_KEY, !prev);
      return !prev;
    });
  }, []);

  const jump = useCallback(
    (line: number) => {
      if (selectedFile !== undefined) onNavigate?.(selectedFile.path, line);
    },
    [selectedFile, onNavigate]
  );

  return (
    <div className="codect-view">
      <div className="codect-controls">
        <button
          type="button"
          className="codect-tree-toggle"
          aria-pressed={!treeCollapsed}
          title={t("codect.label.toggleTree")}
          onClick={toggleTree}
        >
          ▤
        </button>
        <span className="codect-field">
          <input
            className="codect-input"
            type="text"
            placeholder={t("codect.placeholder.revision")}
            value={revision}
            onChange={(e) => setRevision(e.target.value)}
            title={t("codect.label.revision")}
          />
        </span>
        <ModeToggle value={mode} onChange={setMode} label={t("codect.label.mode")} />
        <span className="codect-field">
          <input
            className="codect-input"
            type="text"
            placeholder={t("codect.placeholder.path")}
            value={pathFilter}
            onChange={(e) => setPathFilter(e.target.value)}
            title={t("codect.label.pathFilter")}
          />
        </span>
      </div>

      {loading && <div className="codect-loading">{t("codect.loading.show")}</div>}
      {error && <div className="codect-error">{error}</div>}

      {document !== null && !loading && files.length === 0 && (
        <div className="codect-empty">{t("codect.empty.show")}</div>
      )}

      {document !== null && !loading && files.length > 0 && (
        <SplitPane
          storageKey="codect.show.treeWidth"
          tree={
            treeCollapsed ? null : (
              <TreeView
                rows={rows}
                selectedPath={selectedPath}
                onSelect={setSelectedPath}
                onToggleDir={toggleDir}
              />
            )
          }
          content={
            selectedFile !== undefined && (
              <div className="codect-content">
                <div className="codect-file-header">
                  <span className="codect-file-path" title={selectedFile.path}>
                    {selectedFile.path}
                  </span>
                  <span className="codect-file-meta">
                    {selectedFile.language} · {selectedFile.outline.length}{" "}
                    {t("codect.label.declarations")}
                  </span>
                </div>

                <div className="codect-section">
                  <button
                    type="button"
                    className="codect-section-header"
                    aria-expanded={outlineOpen}
                    onClick={toggleOutline}
                  >
                    <span className="codect-section-chevron">{outlineOpen ? "▾" : "▸"}</span>
                    <span className="codect-section-title">{t("codect.label.outline")}</span>
                    <span className="codect-section-meta">{outline.length}</span>
                  </button>
                  {outlineOpen && (
                    <div className="codect-section-body">
                      <OutlineView rows={outline} onJump={jump} />
                    </div>
                  )}
                </div>

                <CodePane
                  text={selectedFile.projection.text}
                  language={selectedFile.language}
                  title={selectedFile.path}
                  labels={labels}
                />
              </div>
            )
          }
        />
      )}
    </div>
  );
}
