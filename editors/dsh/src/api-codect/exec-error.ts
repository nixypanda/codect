/** Classify an execFile failure without importing Node or DSH types. */
export interface ExecFailure {
  code?: string | number | null;
  killed?: boolean;
  signal?: string | null;
}

export type ExecFailureKind = "binary-missing" | "timeout" | "failed";

/**
 * `execFile` reports a missing binary as ENOENT and a timeout as a killed
 * child with a signal; everything else is a plain failure.
 */
export function classifyExecError(error: ExecFailure): ExecFailureKind {
  if (error.code === "ENOENT") return "binary-missing";
  if (error.killed || error.signal) return "timeout";
  return "failed";
}
