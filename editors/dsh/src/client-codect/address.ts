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

/** Component-encode one id or path segment, keeping `:` literal for drives. */
function encodeSegment(segment: string): string {
  return encodeURIComponent(segment).replace(/%3A/gi, ":");
}

/** Encode a `/`-separated path segment by segment. */
function encodePath(path: string): string {
  return path.split("/").map(encodeSegment).join("/");
}

function isAbsolutePath(path: string): boolean {
  return path.startsWith("/") || path.startsWith("//") || /^[A-Za-z]:[\\/]/.test(path);
}

/** The `dsh-resource://file/session/<id>/<path>` address for one session. */
export function sessionFileAddress(sessionId: string, path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/^(?:\.\/)+/, "");
  return `${FILE_ADDRESS_PREFIX}session/${encodeSegment(sessionId)}/${encodePath(normalized)}`;
}

/**
 * The address for a path as a caller holds it: a relative path, or an absolute
 * path inside the session workspace, becomes a `session`-scoped address.
 *
 * @param sessionId - the session the path is read in.
 * @param cwd - the session workspace root, when known.
 * @param path - the repo-relative or absolute path.
 */
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
