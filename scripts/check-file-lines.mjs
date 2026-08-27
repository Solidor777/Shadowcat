// Fails when a covered source file exceeds the line limits.
//
// A file that no longer fits in one reading is edited by pattern-matching on fragments of it, and
// every reviewer of it does the same; the defect rate of such edits is the reason for the limits.
// The soft limit (5,000) is the real line: crossing it fails unless the repository owner has
// approved that specific file in the allowlist named below. The hard limit (10,000) has no
// override. Neither limit grandfathers anything: an entry whose file has since dropped to the
// limit fails in its own right, so permission cannot accumulate.
//
// Test lines count. Splitting a test module into its own file is the intended remedy, and
// `check-inline-tests.mjs` enforces that Rust test modules live in sibling files.
//
// Enumeration is `git ls-files`, so untracked scratch and build output never count.

import { readFileSync, existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { extname } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { GENERATED_ROOT, norm, under } from "./lib/gate-corpus.mjs";

export const SOFT_LIMIT = 5000;
export const HARD_LIMIT = 10000;
export const ROOTS = ["src", "scripts", "examples"];
export const COVERED_EXTS = new Set([".rs", ".ts", ".js", ".mjs", ".svelte", ".scss", ".css"]);
export const ALLOWLIST = ".claude/file-size-allowlist.toml";

/** Newline count, identical to `wc -l`, plus one for an unterminated final line. */
export function countLines(text) {
  if (text.length === 0) return 0;
  let n = 0;
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) n++;
  return text.endsWith("\n") ? n : n + 1;
}

/** Whether `path` (any separator) is a file the limits apply to. */
export function isCovered(path) {
  const p = norm(path);
  if (!ROOTS.some((r) => under(p, r))) return false;
  if (under(p, GENERATED_ROOT)) return false;
  return COVERED_EXTS.has(extname(p));
}

/**
 * Parses the allowlist. Accepts only `[[file]]` tables of double-quoted string values and throws
 * on anything else: a line silently ignored here would be an approval nobody granted.
 */
export function parseAllowlist(text, sourceName) {
  const out = [];
  let cur = null;
  text.split("\n").forEach((raw, i) => {
    const line = raw.replace(/\r$/, "").trim();
    if (line === "" || line.startsWith("#")) return;
    if (line === "[[file]]") {
      cur = { path: "", lines_at_approval: "", reason: "" };
      out.push(cur);
      return;
    }
    const m = line.match(/^(path|lines_at_approval|reason)\s*=\s*"((?:[^"\\]|\\.)*)"$/);
    if (!m || cur === null) {
      throw new Error(
        `${sourceName}:${i + 1}: cannot parse. Expected [[file]] or path/lines_at_approval/reason = "value". got: ${raw}`
      );
    }
    cur[m[1]] = m[2].replace(/\\"/g, '"');
  });
  return out;
}

/** Applies the limits to measured files; pure so the suite can drive every branch. */
export function evaluate({ files, allow }) {
  const errors = [];
  const allowed = new Set(allow.map((a) => norm(a.path)));
  const measured = new Map(files.map((f) => [norm(f.path), f.lines]));
  for (const [path, lines] of measured) {
    if (lines > HARD_LIMIT) {
      errors.push({
        kind: "HARD LIMIT",
        path,
        lines,
        message: `${path}: ${lines} lines exceeds the hard limit of ${HARD_LIMIT}. No override exists; split the file.`,
      });
    } else if (lines > SOFT_LIMIT && !allowed.has(path)) {
      errors.push({
        kind: "SOFT LIMIT",
        path,
        lines,
        message: `${path}: ${lines} lines exceeds the soft limit of ${SOFT_LIMIT}. Split the file, or obtain the repository owner's explicit approval and record it in ${ALLOWLIST}.`,
      });
    }
  }
  for (const a of allow) {
    const path = norm(a.path);
    const lines = measured.get(path);
    if (lines === undefined || lines <= SOFT_LIMIT) {
      errors.push({
        kind: "STALE ALLOWLIST ENTRY",
        path,
        lines: lines ?? 0,
        message: `${path}: allowlist entry is stale (${
          lines === undefined ? "file is not tracked" : `${lines} lines, at or under ${SOFT_LIMIT}`
        }). Remove the entry.`,
      });
    }
  }
  return errors;
}

/** Tracked paths under the roots, from git so build output and scratch never count. */
function trackedFiles() {
  const out = execFileSync("git", ["ls-files", "-z", "--", ...ROOTS], { encoding: "utf8" });
  return out.split("\0").filter((p) => p.length > 0);
}

function main() {
  const files = trackedFiles()
    .filter(isCovered)
    .map((path) => ({ path: norm(path), lines: countLines(readFileSync(path, "utf8")) }));
  let allow = [];
  if (existsSync(ALLOWLIST)) {
    try {
      allow = parseAllowlist(readFileSync(ALLOWLIST, "utf8"), ALLOWLIST);
    } catch (e) {
      console.error(e.message);
      process.exit(2);
    }
  }
  const errors = evaluate({ files, allow });
  for (const e of errors) console.error(`${e.kind}: ${e.message}`);
  console.log(`lint:file-size: ${files.length} files measured, ${allow.length} allowlisted, ${errors.length} error(s)`);
  process.exit(errors.length === 0 ? 0 : 1);
}

if (isDirectEntry(import.meta.url)) main();
