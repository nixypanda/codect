#!/usr/bin/env node
/**
 * Build script for the codect DSH plugin.
 *
 * Produces lib/client.js in the exact ModuleLoader format that DSH expects:
 *   window.__ModuleLoader__.load({ id: "...", factory: (require) => { ... } })
 */

import { rmSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");
const libDir = join(root, "lib");
const srcDir = join(root, "src");
const esbuild = process.env.ESBUILD_BINARY || "esbuild";

rmSync(libDir, { recursive: true, force: true });
mkdirSync(libDir, { recursive: true });
mkdirSync(join(libDir, "types"), { recursive: true });
mkdirSync(join(libDir, "types", "client"), { recursive: true });

function build(args) {
  execSync(`${esbuild} ${args}`, { stdio: "inherit" });
}

// 1. Host service
console.log("Building host service...");
build(`${join(srcDir, "api-codect/index.ts")} --bundle --format=esm --platform=node --target=node22 --packages=external --outfile=${join(libDir, "index.js")}`);

// 2. Typert host
console.log("Building typert.host.js...");
build(`${join(srcDir, "api-codect/typert.host.ts")} --bundle --format=esm --platform=node --target=node22 --packages=external --outfile=${join(libDir, "typert.host.js")}`);

// 3. Client — build as CJS, then wrap in ModuleLoader format. React and the
// JSX runtime come from the browser platform table; relative imports (views,
// definition, the Remote contribution) are bundled.
console.log("Building client.js...");
build(`${join(srcDir, "client-codect/client.ts")} --bundle --format=cjs --platform=browser --target=es2022 --packages=external --outfile=${join(libDir, "client.tmp.js")}`);

// Wrap the CJS output in ModuleLoader format. The CJS bundle assigns its named
// exports (apply, inject, ...) onto `module.exports`, and the factory returns
// that object to DSH.
const cjs = readFileSync(join(libDir, "client.tmp.js"), "utf8");
const wrapper = `window.__ModuleLoader__.load({
  id: "@nixypanda/dsh-codect",
  factory: (require) => {
    var module = { exports: {} };
    var exports = module.exports;
${cjs}
    return module.exports;
  },
});`;
writeFileSync(join(libDir, "client.js"), wrapper);
rmSync(join(libDir, "client.tmp.js"));

// 4. Shared schema
console.log("Building shared schema...");
build(`${join(srcDir, "shared/schema.ts")} --bundle --format=esm --platform=browser --packages=external --outfile=${join(libDir, "types/types.js")}`);

// 5. Type declarations
console.log("Writing type declarations...");
writeFileSync(join(libDir, "types/index.d.ts"), `export { CodectService } from "../../src/api-codect/index";
export type { ShowDocument, DiffDocument, CodectMode } from "../../src/shared/schema";
`);
writeFileSync(join(libDir, "types/client/index.d.ts"), `export {};
`);
writeFileSync(join(libDir, "typert.host.d.ts"), `export const TYPERT: any;
`);

console.log("Build complete → lib/");
