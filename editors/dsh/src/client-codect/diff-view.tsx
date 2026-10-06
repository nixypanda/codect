/**
 * diff-view.tsx — Codect "Diff" view.
 *
 * A collapsible changed-file tree on the left beside a unified projection diff
 * (`DiffBlock`). A secondary toggle shows the declaration-level outline
 * comparison, derived by matching the two sides' outlines on `stable_key`.
 * Side by side when the pane is wide enough, stacked when it is not.
 */

import React, { useCallback, useEffect, useMemo, useState } from "react";
import type {
  CodectMode,
  DiffChange,
  DiffDocument,
  DiffFile,
  OutlineItem,
} from "../shared/schema";
import type { CodeLabels } from "./labels";
import { buildTreeRows } from "./tree";
import { DiffBody, type DiffLayout } from "./components/diff-body";
import { ModeToggle } from "./components/mode-toggle";
import { TreeView, type FileStatus } from "./components/tree-view";
import { SplitPane } from "./components/split-pane";
import { SegmentedControl } from "./primitives";

export interface DiffViewProps {
  root: string;
  codect: {
    diff: (params: {
      root: string;
      base: string;
      target: string;
      mode: CodectMode;
      paths?: string[];
      areas?: string[];
    }) => Promise<DiffDocument>;
  };
  labels: CodeLabels;
  t: (key: string) => string;
  onNavigate?: (path: string, line: number) => void;
  /**
   * Optional `dsh-diff-view` client service. When present, the diff body can
   * render side by side; when absent the layout control is hidden.
   */
  diffView?: any;
}

/** Compare the two sides of a file's outline into declaration-level changes. */
export function diffChanges(file: DiffFile): DiffChange[] {
  const baseItems = new Map<string, OutlineItem>(
    (file.base?.outline ?? []).map((item) => [item.stable_key, item])
  );
  const targetItems = new Map<string, OutlineItem>(
    (file.target?.outline ?? []).map((item) => [item.stable_key, item])
  );

  const changes: DiffChange[] = [];
  for (const [key, item] of baseItems) {
    if (!targetItems.has(key)) {
      changes.push({ status: "deleted", item, base_text: item.signature, target_text: null });
    }
  }
  for (const [key, item] of targetItems) {
    if (!baseItems.has(key)) {
      changes.push({ status: "added", item, base_text: null, target_text: item.signature });
    }
  }
  for (const [key, item] of targetItems) {
    const before = baseItems.get(key);
    if (before !== undefined && before.signature !== item.signature) {
      changes.push({ status: "changed", item, base_text: before.signature, target_text: item.signature });
    }
  }
  return changes;
}

const MARKER: Record<DiffChange["status"], string> = {
  added: "+",
  deleted: "−",
  changed: "~",
};

const TREE_COLLAPSED_KEY = "codect.diff.treeCollapsed";
const LAYOUT_KEY = "codect.diff.layout";
const SNAPSHOT_LIST_ID = "codect-snapshot-sides";

/**
 * The mutable snapshot sides a `diff` accepts besides commits. They are
 * resolved by the CLI against the working repository, so `:worktree` reflects
 * tracked and untracked, non-ignored files on disk and `:index` the staged
 * blobs. Exposed as datalist options on both revision inputs.
 */
const SNAPSHOT_SIDES = [":index", ":worktree", ":empty"];

/**
 * One-click comparisons over the common snapshot pairs: uncommitted changes
 * (`HEAD → :worktree`), staged changes (`HEAD → :index`), unstaged changes
 * (`:index → :worktree`), and the whole non-ignored tree (`:empty → :worktree`).
 */
const SNAPSHOT_PRESETS: { id: string; base: string; target: string }[] = [
  { id: "worktree", base: "HEAD", target: ":worktree" },
  { id: "index", base: "HEAD", target: ":index" },
  { id: "staged", base: ":index", target: ":worktree" },
  { id: "empty", base: ":empty", target: ":worktree" },
];

function readTreeCollapsed(): boolean {
  try {
    return globalThis.localStorage?.getItem(TREE_COLLAPSED_KEY) === "true";
  } catch {
    return false;
  }
}

function readLayout(): DiffLayout {
  try {
    return globalThis.localStorage?.getItem(LAYOUT_KEY) === "split" ? "split" : "unified";
  } catch {
    return "unified";
  }
}

function writeLayout(layout: DiffLayout): void {
  try {
    globalThis.localStorage?.setItem(LAYOUT_KEY, layout);
  } catch {
    // Persisting the layout is best-effort.
  }
}

function Declarations({
  file,
  onNavigate,
}: {
  file: DiffFile;
  onNavigate?: (path: string, line: number) => void;
}) {
  const changes = useMemo(() => diffChanges(file), [file]);
  if (changes.length === 0) {
    return <div className="codect-empty">No declaration changes</div>;
  }
  return (
    <div className="codect-outline">
      {changes.map((change) => (
        <div
          key={`${change.status}:${change.item.stable_key}`}
          className={`codect-outline-row codect-decl-${change.status}`}
          title={change.target_text ?? change.base_text ?? change.item.name}
          onClick={() => onNavigate?.(file.path, change.item.span.start_line)}
        >
          <span className="codect-decl-marker">{MARKER[change.status]}</span>
          <span className="codect-outline-kind">{change.item.kind}</span>
          <span className="codect-outline-name">{change.item.name}</span>
          <span className="codect-outline-sig">{change.target_text ?? change.base_text ?? ""}</span>
        </div>
      ))}
    </div>
  );
}

type DiffViewMode = "diff" | "declarations";

export function DiffView({ root, codect, labels, t, onNavigate, diffView }: DiffViewProps) {
  const [document, setDocument] = useState<DiffDocument | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [base, setBase] = useState("HEAD~1");
  const [target, setTarget] = useState("HEAD");
  const [mode, setMode] = useState<CodectMode>("types");
  const [pathFilter, setPathFilter] = useState("");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<DiffViewMode>("diff");
  const [layout, setLayout] = useState<DiffLayout>(() => readLayout());
  const [treeCollapsed, setTreeCollapsed] = useState<boolean>(() => readTreeCollapsed());

  // Highlighted rendering (both layouts) comes from the optional `dsh-diff-view`
  // client service; hide the layout control entirely when it is not installed.
  const diffViewAvailable =
    diffView !== null &&
    diffView !== undefined &&
    typeof diffView.diffFileComponent === "function";

  const changeLayout = useCallback((next: DiffLayout) => {
    setLayout(next);
    writeLayout(next);
  }, []);

  const loadDiff = useCallback(async () => {
    if (!root || !base || !target) return;
    setLoading(true);
    setError(null);
    try {
      const doc = await codect.diff({
        root,
        base,
        target,
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
  }, [root, base, target, mode, pathFilter, codect]);

  useEffect(() => {
    void loadDiff();
  }, [loadDiff]);

  const files = document?.files ?? [];
  const rows = useMemo(
    () => buildTreeRows(files.map((file) => file.path), collapsed),
    [files, collapsed]
  );
  const statusByPath = useMemo(
    () => new Map(files.map((file) => [file.path, file.status as FileStatus])),
    [files]
  );
  const statusOf = useCallback(
    (path: string): FileStatus | undefined => statusByPath.get(path),
    [statusByPath]
  );
  const selectedFile: DiffFile | undefined = files.find(
    (file) => file.path === selectedPath
  );

  const toggleDir = useCallback((path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const toggleTree = useCallback(() => {
    setTreeCollapsed((prev) => {
      try {
        globalThis.localStorage?.setItem(TREE_COLLAPSED_KEY, String(!prev));
      } catch {
        // best-effort
      }
      return !prev;
    });
  }, []);

  const viewOptions = [
    { value: "diff" as DiffViewMode, label: t("codect.label.diff") },
    { value: "declarations" as DiffViewMode, label: t("codect.label.declarations") },
  ];

  const layoutOptions = [
    { value: "unified" as DiffLayout, label: t("codect.label.unified") },
    { value: "split" as DiffLayout, label: t("codect.label.sideBySide") },
  ];

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
            placeholder={t("codect.placeholder.base")}
            value={base}
            list={SNAPSHOT_LIST_ID}
            onChange={(e) => setBase(e.target.value)}
            title={t("codect.label.base")}
          />
        </span>
        <ModeToggle value={mode} onChange={setMode} label={t("codect.label.mode")} />
        <span className="codect-field">
          <input
            className="codect-input"
            type="text"
            placeholder={t("codect.placeholder.target")}
            value={target}
            list={SNAPSHOT_LIST_ID}
            onChange={(e) => setTarget(e.target.value)}
            title={t("codect.label.target")}
          />
        </span>
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

      <div className="codect-snapshots" role="group" aria-label={t("codect.label.snapshots")}>
        <span className="codect-snapshots-label">{t("codect.label.snapshots")}</span>
        {SNAPSHOT_PRESETS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            className="codect-snapshot"
            aria-pressed={base === preset.base && target === preset.target}
            title={`${preset.base} → ${preset.target}`}
            onClick={() => {
              setBase(preset.base);
              setTarget(preset.target);
            }}
          >
            {t(`codect.preset.${preset.id}`)}
          </button>
        ))}
      </div>
      <datalist id={SNAPSHOT_LIST_ID}>
        {SNAPSHOT_SIDES.map((side) => (
          <option key={side} value={side} />
        ))}
      </datalist>

      {loading && <div className="codect-loading">{t("codect.loading.diff")}</div>}
      {error && <div className="codect-error">{error}</div>}

      {document !== null && !loading && (
        <>
          <div className="codect-summary">
            <code>{document.base.revision}</code>
            <span> → </span>
            <code>{document.target.revision}</code>
            <span> · {document.files.length} {t("codect.label.files")}</span>
          </div>
          {files.length === 0 ? (
            <div className="codect-empty">{t("codect.empty.diff")}</div>
          ) : (
            <SplitPane
              storageKey="codect.diff.treeWidth"
              tree={
                treeCollapsed ? null : (
                  <TreeView
                    rows={rows}
                    selectedPath={selectedPath}
                    onSelect={setSelectedPath}
                    onToggleDir={toggleDir}
                    statusOf={statusOf}
                  />
                )
              }
              content={
                selectedFile !== undefined && (
                  <div className="codect-content">
                    <div className="codect-toolbar-row">
                      <SegmentedControl
                        className="codect-segmented"
                        id="codect-diff-view"
                        value={viewMode}
                        options={viewOptions}
                        onChange={(next: DiffViewMode) => setViewMode(next)}
                        label={t("codect.label.diffView")}
                      />
                      {diffViewAvailable && (
                        <SegmentedControl
                          className="codect-segmented"
                          id="codect-diff-layout"
                          value={layout}
                          options={layoutOptions}
                          onChange={(next: DiffLayout) => changeLayout(next)}
                          label={t("codect.label.diffLayout")}
                        />
                      )}
                    </div>
                    {viewMode === "diff" ? (
                      <DiffBody
                        file={selectedFile}
                        labels={labels}
                        layout={diffViewAvailable ? layout : "unified"}
                        diffView={diffView}
                      />
                    ) : (
                      <Declarations file={selectedFile} onNavigate={onNavigate} />
                    )}
                  </div>
                )
              }
            />
          )}
        </>
      )}
    </div>
  );
}
