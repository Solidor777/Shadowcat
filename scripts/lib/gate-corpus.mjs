// What the documentation gates scan, and the one exemption marker they both honour.
//
// The comment gate and the skill-symbol-citation gate each need the other's answer to some of
// these: the comment gate scopes its skill corpus by tracked-ness, which the symbol gate computes,
// while the symbol gate needs the path primitives and the specimen marker the comment gate owns.
// Importing across gates satisfies that but forms a cycle: whichever module is the entry point
// evaluates second, so a module-scope constant derived from an imported binding dies in the
// temporal dead zone, in the gate that did NOT introduce it. Both gates import DOWNWARD from here
// instead, which is the same reason the comment-span splitter already lives beside this file.

import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

// Shared: both gates walk the same tree and must skip the same directories. Two copies of a skip
// list drift into two different notions of what the repo is.
export const SKIP_DIRS = new Set([
  "node_modules",
  "dist",
  "target",
  ".git",
  "dist-docs",
]);

// ts-rs output, never hand-written — the owner ruled this whole population out of scope. Excluded
// by path prefix rather than folded into `SKIP_DIRS` (a directory-NAME skip that would blindly
// exclude any directory named "generated" anywhere in the tree), matching the shape
// eslint.config.js's `"src/types/generated/"` ignore already uses for the same reasoning. The
// exclusion covers the directory, not a content heuristic — a hand-written file could carry a
// banner comment too, and this rule must not exempt that.
export const GENERATED_ROOT = "src/types/generated";

// The codebase-skill briefs are prose about the code, not code. A separate root from the code roots
// because these files carry a different extension and a different notion of "comment" — the whole
// line is prose, there is no surrounding code to split it from.
export const MD_ROOTS = [".claude/skills"];
export const MD_EXTS = [".md"];

/** Repo-relative path with forward slashes, so a scope reads the same on every platform. */
export const norm = (p) => p.split("\\").join("/");

// Prefix matching is path-boundary-aware: a raw `startsWith` makes "src/modules/chat" also claim
// "src/modules/chat-card", silently pulling a sibling directory into a scope that never named it.
// The over-match is invisible — the count is simply larger, and larger reads as more thorough.
// Shared, because both gates resolve the same path-prefix question over the same tree, and a
// second copy of a boundary-aware prefix test is a second place for the boundary rule to be got
// wrong.
export const under = (p, prefix) => p === norm(prefix) || p.startsWith(norm(prefix) + "/");

/** Recursively collects paths matching `exts` under `dir`; called once per entry in a roots list.
 * @param {string} dir - directory to walk.
 * @param {string[]} exts - file extensions to keep.
 * @returns {string[]} matching paths.
 */
export function sources(dir, exts) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sources(p, exts));
    else if (exts.some((e) => name.endsWith(e))) out.push(p);
  }
  return out;
}

// The one exemption either gate has: a line deliberately exhibiting a form in order to DEFINE it
// is not a real instance of it, so neither gate may read it as one. The marker sits on the line it
// exempts, so it has no position to rot, and each gate prints its active count — an uncounted
// exemption is a backdoor, and a silent one is indistinguishable from a rule that does not apply.
//
// Shared rather than copied: two copies would be two decisions about what "exempt" means, free to
// disagree. It exempts the WHOLE line, so a genuine violation sharing a line with a specimen is
// hidden — write the specimen on a line of its own.
export const EXAMPLE_EXEMPT = /\bEXAMPLE:/;

/**
 * The skill directories git TRACKS — the corpus both gates govern. Tracked-ness is the property
 * that actually matters: a directory holding no committed file is vendored third-party content
 * this repo neither wrote nor maintains, and its prose documents an external tool's own API
 * rather than Shadowcat's. Scoping by a directory-NAME pattern instead would encode the wrong
 * reason and silently drop a future first-party skill named anything else.
 *
 * Shared so the two gates cannot disagree about the size of the corpus; each prints the excluded
 * count on every run.
 *
 * @param {string} repoRoot - absolute path to the repository root.
 * @returns {{ tracked: Set<string>, untracked: string[] }|null} tracked and untracked directory
 *   names under the skill roots, or null when git cannot answer (no checkout, or no git).
 */
export function listSkillDirs(repoRoot) {
  const tracked = new Set();
  for (const root of MD_ROOTS) {
    let listed;
    try {
      listed = execFileSync("git", ["ls-files", "-z", "--", root], {
        cwd: repoRoot,
        encoding: "utf8",
        maxBuffer: 64 * 1024 * 1024,
      });
    } catch {
      return null;
    }
    for (const entry of listed.split("\0")) {
      if (entry === "") continue;
      const rel = norm(entry).slice(norm(root).length + 1);
      if (rel.includes("/")) tracked.add(rel.split("/")[0]);
    }
  }
  const untracked = [];
  for (const root of MD_ROOTS) {
    const abs = join(repoRoot, ...root.split("/"));
    for (const entry of readdirSync(abs, { withFileTypes: true })) {
      if (entry.isDirectory() && !tracked.has(entry.name)) untracked.push(entry.name);
    }
  }
  return { tracked, untracked };
}
