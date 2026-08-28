// Empties the build-output directories, recoverably, before a build.
//
// Every output directory is listed here by name and nothing else can be a target: the remover
// asserts each resolved path against this list before touching it, so a pattern typo or a stray
// argument cannot reach `src/`, `docs/`, or the repository root. Removal goes to the OS recycle
// bin / trash rather than an unlink, so a wrong target is recoverable at the moment it happens.
//
// This is the single place a build-output directory is registered; a caller that needs its own
// output cleared (Vite's `emptyOutDir`, the docs assembler) reuses `removeRecoverably` instead of
// clearing its own tree directly.

import { existsSync, readdirSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import process from "node:process";
import trash from "trash";
import { isDirectEntry } from "./lib/is-main.mjs";
import { norm } from "./lib/gate-corpus.mjs";

export const TARGETS = ["dist", "dist-docs", "docs/site/.vitepress/dist", "examples/*/dist", "target/package"];

const ALLOWED = /^(dist|dist-docs|docs\/site\/\.vitepress\/dist|examples\/[^/]+\/dist|target\/package)$/;

/** Throws unless `absPath` is one of the enumerated output directories under `root`. */
export function assertAllowed(root, absPath) {
  const rel = norm(relative(resolve(root), resolve(absPath)));
  if (rel === "" || rel.startsWith("..") || resolve(absPath) !== resolve(root, rel) || !ALLOWED.test(rel)) {
    throw new Error(`clean-build-outputs: refusing to remove '${absPath}' — not an enumerated build-output directory`);
  }
}

/** Existing directories matching the patterns (only `examples/*` expands per example; every other pattern maps to one directory). */
export function resolveTargets(root, patterns) {
  const out = [];
  for (const pat of patterns) {
    if (!TARGETS.includes(pat)) throw new Error(`clean-build-outputs: refusing unlisted pattern '${pat}'`);
    if (pat === "examples/*/dist") {
      const ex = join(root, "examples");
      if (!existsSync(ex)) continue;
      for (const name of readdirSync(ex)) {
        const p = join(ex, name, "dist");
        if (existsSync(p) && statSync(p).isDirectory()) out.push(p);
      }
    } else {
      const p = join(root, ...pat.split("/"));
      if (existsSync(p) && statSync(p).isDirectory()) out.push(p);
    }
  }
  return out;
}

/** Sends a path to the OS recycle bin / trash. */
export async function removeRecoverably(absPath) {
  await trash(absPath, { glob: false });
}

/** Resolves, asserts, then removes; returns the removed paths. Nothing is removed if any assertion fails. */
export async function clean({ root, patterns, remove = removeRecoverably }) {
  const targets = resolveTargets(root, patterns);
  for (const t of targets) assertAllowed(root, t);
  for (const t of targets) await remove(t);
  return targets;
}

async function main() {
  const root = resolve(process.cwd());
  const onlyIdx = process.argv.indexOf("--only");
  const patterns = onlyIdx >= 0 ? [process.argv[onlyIdx + 1]] : TARGETS;
  const removed = await clean({ root, patterns });
  for (const p of removed) console.log(`trashed ${norm(relative(root, p))}`);
  console.log(`clean: ${removed.length} output director${removed.length === 1 ? "y" : "ies"} sent to trash`);
}

if (isDirectEntry(import.meta.url)) main().catch((e) => { console.error(e.message); process.exit(1); });
