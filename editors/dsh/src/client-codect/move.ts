/**
 * move.ts — Pure path helpers for moved files in a codect.diff.v2 document.
 *
 * `path` is always the target-side display/selection key. A moved file also
 * carries `old_path`, its source path. Added files have no base side, deleted
 * files have no target side, and modified files keep the same path on both
 * sides.
 */

import type { DiffChange, DiffFile } from "../shared/schema";

/** How a serialized path string must be interpreted before host lookup. */
export type NavigationPathEncoding =
  | "codect.path.v1"
  | "legacy"
  | "unsupported";

/** diff v2 always uses the fixed display encoding; it carries no marker. */
export const DIFF_PATH_ENCODING = "codect.path.v1" as const;

/**
 * The pre-marker show encoding escaped invalid UTF-8 bytes as `\xNN` but left a
 * literal backslash raw. A legacy path containing an escaped high byte is
 * therefore ambiguous between that invalid byte and a literal filename, so it
 * must not be opened as either.
 */
const LEGACY_AMBIGUOUS_ESCAPE = /\\x[89A-F][0-9A-F]/;

/**
 * Read a show document's additive path-encoding marker. Missing means the
 * pre-marker show encoding; any unknown marker is unsupported, never decoded as
 * the current encoding.
 */
export function showNavigationEncoding(
  document: { path_encoding?: string } | null | undefined
): NavigationPathEncoding {
  const marker = document?.path_encoding;
  if (marker === "codect.path.v1") return "codect.path.v1";
  if (marker === undefined) return "legacy";
  return "unsupported";
}

/** A legacy path is usable unless it may be an escaped invalid UTF-8 byte. */
export function legacyNavigationPath(path: string): string | null {
  return LEGACY_AMBIGUOUS_ESCAPE.test(path) ? null : path;
}

/** Decode a serialized path exactly once for the encoding that produced it. */
export function navigationPath(
  path: string,
  encoding: NavigationPathEncoding
): string | null {
  if (encoding === "codect.path.v1") return decodeDisplayPath(path);
  if (encoding === "legacy") return legacyNavigationPath(path);
  return null;
}

export function canNavigatePath(
  path: string,
  encoding: NavigationPathEncoding
): boolean {
  return navigationPath(path, encoding) !== null;
}

/**
 * Decode Codect's injective path display encoding back to a raw JavaScript
 * path string, or null when the display names bytes JavaScript cannot carry.
 *
 * The canonical encoder doubles a literal backslash, uses `\n`, `\r`, `\t` for
 * those controls, and `\xNN` for other ASCII controls, DEL, and invalid UTF-8
 * bytes. Ordinary Unicode is unchanged. `\xNN` above 0x7F is necessarily an
 * invalid UTF-8 byte, so no JavaScript string can represent the original path;
 * returning null disables navigation rather than opening a substituted file.
 */
export function decodeDisplayPath(display: string): string | null {
  let decoded = "";

  const hex = (code: number): number => {
    if (code >= 0x30 && code <= 0x39) return code - 0x30;
    if (code >= 0x41 && code <= 0x46) return code - 0x41 + 10;
    if (code >= 0x61 && code <= 0x66) return code - 0x61 + 10;
    return -1;
  };

  for (let index = 0; index < display.length; index += 1) {
    const character = display[index];
    if (character !== "\\") {
      decoded += character;
      continue;
    }

    const escape = display[index + 1];
    if (escape === "\\") {
      decoded += "\\";
      index += 1;
      continue;
    }
    if (escape === "n") {
      decoded += "\n";
      index += 1;
      continue;
    }
    if (escape === "r") {
      decoded += "\r";
      index += 1;
      continue;
    }
    if (escape === "t") {
      decoded += "\t";
      index += 1;
      continue;
    }
    if (escape !== "x") return null;

    const high = hex(display.charCodeAt(index + 2));
    const low = hex(display.charCodeAt(index + 3));
    if (high < 0 || low < 0) return null;
    const byte = (high << 4) | low;
    if (byte >= 0x80) return null;

    decoded += String.fromCharCode(byte);
    index += 3;
  }

  return decoded;
}

/** The path for the base side of an event, or null when that side is absent. */
export function basePath(file: DiffFile): string | null {
  if (file.base === null) return null;
  return file.status === "moved" ? file.old_path : file.path;
}

/** The path for the target side of an event, or null when that side is absent. */
export function targetPath(file: DiffFile): string | null {
  return file.target === null ? null : file.path;
}

/** The source path of a move, or undefined for every other status. */
export function movedFrom(file: DiffFile): string | undefined {
  return file.status === "moved" ? file.old_path : undefined;
}

/** `old → new` for a move, or undefined for every other status. */
export function moveLabel(file: DiffFile): string | undefined {
  return file.status === "moved" ? `${file.old_path} → ${file.path}` : undefined;
}

/** The source file to open for a declaration-level change. */
export function pathForChange(
  file: DiffFile,
  status: DiffChange["status"]
): string {
  return status === "deleted" ? basePath(file) ?? file.path : file.path;
}

/**
 * A moved file's deleted declaration exists only on the base side. The DSH
 * navigator opens workspace files at the target, where that source path no
 * longer exists, so leave the row visible but non-clickable instead of opening
 * a dead target or the wrong source line. Other deleted declarations keep the
 * preexisting navigation behavior.
 */
export function canNavigateChange(
  file: DiffFile,
  status: DiffChange["status"]
): boolean {
  return !(file.status === "moved" && status === "deleted");
}
