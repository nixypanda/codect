/**
 * index.ts — Backend cordis service for the codect DSH plugin.
 *
 * Runs in the Electron main process. Extends TypertRemoteService so the
 * @Remote methods are registered on the Typert gateway. Pattern follows
 * dsh-api-workspace-files/lib/index.js.
 */

import { execFile } from "node:child_process";
import { TypertRemoteService, Remote, RemoteError } from "@deepseek-ai/dsh-typert-protocol";
import z from "@deepseek-ai/schemastery";
import type { ShowDocument, DiffDocument, CodectMode } from "../shared/schema";
import { classifyExecError } from "./exec-error";

// Merge this owner's failure codes into the shared Remote failure vocabulary
// (the documented pattern in `dsh-api-gateway`): without it `code` is not a
// `RemoteErrorCode` and `details` has no type. The values are the structured
// payloads carried on the wire; codect failures carry nothing extra.
declare module "@deepseek-ai/dsh-typert-protocol" {
  interface RemoteErrorDetailsMap {
    "codect/failed": {};
    "codect/invalid-json": {};
    "codect/schema-mismatch": {};
    "codect/invalid-argument": {};
    "codect/binary-missing": {};
    "codect/timeout": {};
  }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const Config = z.object({
  /** Path to the codect binary. Defaults to "codect" on PATH. */
  binary: z.string().default("codect"),
  /** Default timeout for codect invocations, in milliseconds. */
  timeoutMs: z.number().step(1).min(1000).max(120000).default(30000),
});

type ConfigType = Schemastery.TypeT<typeof Config>;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function execCodect(
  binary: string,
  args: string[],
  cwd: string,
  timeout: number,
  expectedSchema?: string
): Promise<unknown> {
  return new Promise((resolve, reject) => {
    execFile(binary, args, { cwd, timeout, maxBuffer: 50 * 1024 * 1024 }, (error, stdout, stderr) => {
      if (error) {
        switch (classifyExecError(error)) {
          case "binary-missing":
            reject(new RemoteError("codect/binary-missing", `${binary} was not found (ENOENT)`, {}));
            return;
          case "timeout":
            reject(new RemoteError("codect/timeout", `codect timed out after ${timeout} ms`, {}));
            return;
          default:
            reject(new RemoteError("codect/failed", stderr?.trim() || `codect exited: ${error.message}`, {}));
            return;
        }
      }
      let doc: unknown;
      try {
        doc = JSON.parse(stdout);
      } catch (e) {
        reject(new RemoteError("codect/invalid-json", `codect returned invalid JSON: ${(e as Error).message}`, {}));
        return;
      }
      if (expectedSchema && (doc as { schema?: string }).schema !== expectedSchema) {
        reject(new RemoteError("codect/schema-mismatch", `expected ${expectedSchema}, got ${(doc as { schema?: string }).schema}`, {}));
        return;
      }
      resolve(doc);
    });
  });
}

/**
 * Reject a positional revision value that begins with `-`. Without this an
 * input such as `--format` (or any other flag) would be consumed by clap as an
 * option rather than as a revision, letting user input alter the command.
 */
function assertRevision(flag: string, value: string): void {
  if (value.startsWith("-")) {
    throw new RemoteError(
      "codect/invalid-argument",
      `${flag} must not start with "-": ${value}`,
      {}
    );
  }
}

/** `--path` and `--area` are mutually exclusive; reject both being present. */
function assertNotBothSelectors(paths?: string[], areas?: string[]): void {
  if (paths?.length && areas?.length) {
    throw new RemoteError(
      "codect/invalid-argument",
      "--path and --area are mutually exclusive",
      {}
    );
  }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

class CodectService extends TypertRemoteService {
  static inject = ["typert"];
  static Config = Config;

  private config: ConfigType;

  constructor(ctx: any, config: ConfigType) {
    super(ctx, "codect");
    this.config = config;
  }

  @Remote
  async show(params: {
    root: string;
    revision?: string;
    mode: CodectMode;
    paths?: string[];
    areas?: string[];
  }): Promise<ShowDocument> {
    const binary = this.config.binary;
    const args = ["show", "--format", "json", "--mode", params.mode];
    assertNotBothSelectors(params.paths, params.areas);
    if (params.paths?.length) {
      for (const p of params.paths) args.push("--path", p);
    }
    if (params.areas?.length) {
      for (const a of params.areas) args.push("--area", a);
    }
    if (params.revision) {
      assertRevision("revision", params.revision);
      args.push("--", params.revision);
    }
    return (await execCodect(binary, args, params.root, this.config.timeoutMs, "codect.show.v1")) as ShowDocument;
  }

  @Remote
  async diff(params: {
    root: string;
    base: string;
    target: string;
    mode: CodectMode;
    paths?: string[];
    areas?: string[];
  }): Promise<DiffDocument> {
    const binary = this.config.binary;
    const args = ["diff", "--format", "json", "--mode", params.mode];
    assertNotBothSelectors(params.paths, params.areas);
    if (params.paths?.length) {
      for (const p of params.paths) args.push("--path", p);
    }
    if (params.areas?.length) {
      for (const a of params.areas) args.push("--area", a);
    }
    assertRevision("base", params.base);
    assertRevision("target", params.target);
    args.push("--", params.base, params.target);
    return (await execCodect(binary, args, params.root, this.config.timeoutMs, "codect.diff.v1")) as DiffDocument;
  }
}

export { CodectService };
export default CodectService;
