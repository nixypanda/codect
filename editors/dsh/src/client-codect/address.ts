/**
 * address.ts — Build `dsh-resource://file/...` addresses for the codect views.
 *
 * Mirrors `fileAddressFor` / `sessionFileAddress` from
 * `@deepseek-ai/dsh-util-workspace-path` (read from the installed app) so the
 * plugin does not need that package at runtime. `codect` file paths are
 * repo-relative, so the common case is a `session`-scoped address.
 *
 * Pure: no React and no DOM.
 */

const FILE_ADDRESS_PREFIX = "dsh-resource://file/";

function encodeSegment(segment: string): string {
  return encodeURIComponent(segment).replace(/%3A/gi, ":");
}

function encodePath(path: string): string {
  return path.split("/").map(encodeSegment).join("/");
}

function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || path.startsWith("//") || /^[A-Za-z]:[\\/]/.test(path);
}

export function sessionFileAddress(sessionId: string, path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/^(?:\.\/)+/, "");
  return `${FILE_ADDRESS_PREFIX}session/${encodeSegment(sessionId)}/${encodePath(normalized)}`;
}

export function fileAddressFor(
  sessionId: string,
  cwd: string | undefined,
  path: string
): string {
  const normalized = path.replace(/\\/g, "/");
  if (!isAbsolutePath(normalized)) return sessionFileAddress(sessionId, normalized);
  const root =
    cwd === undefined ? "" : cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  if (root !== "" && normalized === root) return sessionFileAddress(sessionId, "");
  if (root !== "" && normalized.startsWith(`${root}/`)) {
    return sessionFileAddress(sessionId, normalized.slice(root.length + 1));
  }
  return sessionFileAddress(sessionId, normalized);
}
