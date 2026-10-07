/**
 * split-pane.tsx — Resizable tree/content split for the codect sidebar views.
 *
 * The pane width is persisted per view in localStorage and exposed as the
 * `--codect-tree-width` CSS variable on the `.codect-main` root. The splitter
 * is only interactive when the container query lays the panes out side by
 * side (`display: none` while stacked), so JS never needs to know the layout
 * mode. Passing `tree = null` (the ▤ toggle) hides the pane and splitter.
 */

import React, { useCallback, useRef, useState } from "react";

export interface SplitPaneProps {
  tree: React.ReactNode | null;
  content: React.ReactNode;
  storageKey: string;
  defaultWidth?: number;
  label?: string;
}

const MIN_WIDTH = 140;
const MAX_WIDTH = 480;

function clamp(n: number): number {
  return Math.min(Math.max(n, MIN_WIDTH), MAX_WIDTH);
}

function readNumber(key: string, fallback: number): number {
  try {
    const raw = globalThis.localStorage?.getItem(key);
    if (raw === null || raw === undefined) return fallback;
    const parsed = Number.parseFloat(raw);
    return Number.isFinite(parsed) ? clamp(parsed) : fallback;
  } catch {
    return fallback;
  }
}

function writeNumber(key: string, value: number): void {
  try {
    globalThis.localStorage?.setItem(key, String(value));
  } catch {
    // Persisting the width is best-effort.
  }
}

export function SplitPane({
  tree,
  content,
  storageKey,
  defaultWidth = 240,
  label = "Resize file tree",
}: SplitPaneProps) {
  const [width, setWidth] = useState(() => readNumber(storageKey, defaultWidth));
  const [resizing, setResizing] = useState(false);
  const drag = useRef<{ x: number; w: number } | null>(null);

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      drag.current = { x: e.clientX, w: width };
      setResizing(true);
    },
    [width]
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const active = drag.current;
      if (active === null) return;
      const next = clamp(active.w + (e.clientX - active.x));
      setWidth(next);
      writeNumber(storageKey, next);
    },
    [storageKey]
  );

  const endDrag = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    drag.current = null;
    setResizing(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
  }, []);

  const onDoubleClick = useCallback(() => {
    setWidth(defaultWidth);
    writeNumber(storageKey, defaultWidth);
  }, [defaultWidth, storageKey]);

  return (
    <div
      className="codect-main"
      data-resizing={resizing ? "true" : undefined}
      style={{ ["--codect-tree-width" as string]: `${width}px` } as React.CSSProperties}
    >
      {tree !== null && tree !== undefined && (
        <>
          <div className="codect-tree-pane">{tree}</div>
          <div
            className="codect-splitter"
            role="separator"
            aria-orientation="vertical"
            aria-label={label}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={endDrag}
            onPointerCancel={endDrag}
            onDoubleClick={onDoubleClick}
          />
        </>
      )}
      <div className="codect-content-pane">{content}</div>
    </div>
  );
}
