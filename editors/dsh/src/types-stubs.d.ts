/**
 * Minimal type stubs for standalone development.
 * Module declarations are in node_modules/@types/*. This file only covers
 * globals and node: built-in modules.
 */

// Minimal Node.js child_process stub
declare module "node:child_process" {
  export interface ExecFileException extends Error {
    code?: string | number;
    killed?: boolean;
    signal?: string;
  }
  export function execFile(
    file: string,
    args: string[],
    options: { cwd?: string; timeout?: number; maxBuffer?: number },
    callback: (error: ExecFileException | null, stdout: string, stderr: string) => void
  ): void;
}

// Minimal process global
declare const process: {
  env: Record<string, string | undefined>;
  execArgv: string[];
  versions: { node: string };
};
