#!/usr/bin/env node
/**
 * Build script for the codect DSH plugin.
 *
 * Produces lib/client.js in the exact ModuleLoader format that DSH expects:
 *   window.__ModuleLoader__.load({ id: "...", factory: (require) => { ... } })
 */

import {
  rmSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  readdirSync,
  statSync,
  existsSync,
} from "node:fs";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, relative, resolve, sep } from "node:path";

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
//
// Emit declarations with `tsc` against the committed stubs in scripts/stubs
// instead of hand-writing them. The published tarball ships only lib/, so the
// declarations must be self-contained (no `../../src` specifiers). The
// dedicated tsconfig.build.json maps the runtime packages to local stubs, so
// this resolves with zero node_modules — required by the network-less nix
// build. Only the host entry + shared schema are emitted; the browser client
// is intentionally not declaration-emitted.
console.log("Emitting type declarations...");
execSync(`${process.env.TSC_BINARY || "tsc"} -p tsconfig.build.json`, {
  stdio: "inherit",
  cwd: root,
});

// The client is a browser bundle with no declaration emit; publish an empty
// declaration so `exports["./client"].types` resolves.
writeFileSync(join(libDir, "types/client/index.d.ts"), `export {}\n`);

// 6. Packaging guard — fail the build on declaration/export regressions.
guardPackaging();

function guardPackaging() {
  const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  const exportsMap = pkg.exports || {};

  // (a) Every declared `exports[<subpath>].types` must exist in the build.
  for (const [subpath, entry] of Object.entries(exportsMap)) {
    if (!entry || typeof entry !== "object" || !entry.types) continue;
    const target = resolve(root, entry.types);
    if (!isFile(target)) {
      throw new Error(
        `packaging guard: exports[${JSON.stringify(subpath)}].types points at ` +
          `${entry.types}, which does not exist in the build output`
      );
    }
  }

  // (b) Every relative import/export in a lib/**/*.d.ts must resolve to a
  // declaration/TS file inside lib/. Declarations must never reach out of the
  // shipped tree (e.g. the old broken `../../src` re-exports).
  const specifierRe = /(?:^|[^\w$])(?:from|import)\s*\(?\s*["']([^"']+)["']/g;
  for (const file of walk(libDir).filter((f) => f.endsWith(".d.ts"))) {
    const contents = readFileSync(file, "utf8");
    if (contents.includes("../../src") || contents.includes("../src/")) {
      throw new Error(
        `packaging guard: ${relative(root, file)} references src/ — ` +
          `declarations must be self-contained under lib/`
      );
    }
    for (const match of contents.matchAll(specifierRe)) {
      const spec = match[1];
      if (!spec.startsWith(".")) continue;
      const base = resolve(dirname(file), spec);
      const candidates = [];
      if (/\.(d\.ts|ts)$/.test(spec)) candidates.push(base);
      candidates.push(`${base}.d.ts`, `${base}.ts`, join(base, "index.d.ts"), join(base, "index.ts"));
      const resolved = candidates.find(isFile);
      if (!resolved) {
        throw new Error(
          `packaging guard: ${relative(root, file)} has unresolved relative ` +
            `specifier ${JSON.stringify(spec)}`
        );
      }
      if (relative(libDir, resolved).startsWith("..")) {
        throw new Error(
          `packaging guard: ${relative(root, file)} specifier ${JSON.stringify(spec)} ` +
            `escapes lib/ to ${relative(root, resolved)}`
        );
      }
    }
  }
}

function isFile(path) {
  return existsSync(path) && statSync(path).isFile();
}

function walk(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...walk(full));
    else out.push(full);
  }
  return out;
}

console.log("Build complete → lib/");
