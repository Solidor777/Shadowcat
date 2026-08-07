// Fails when code suppresses a dead-code or unused-item diagnostic.
//
// These two lint families report that something the code declares is not reached. Suppressing one
// does not make the item live — it makes the compiler stop saying so, and the item then reads as
// deliberate to every later reader. The suppression is indistinguishable from a considered design
// decision, which is precisely what it is not unless someone decided it.
//
// A module-scoped suppression is strictly worse than an item-scoped one: it silences every FUTURE
// dead item in that file too, so real dead code accumulates in a place no build will ever report.
//
// A suppression also cannot tell you it has expired. Auditing this repo's eight dead-code
// allowances found three that suppressed nothing at all — the items had since acquired real
// callers, and the annotation stayed behind claiming otherwise. Rust's `#[expect(...)]` inverts
// that: it fails when the lint STOPS firing, so it cannot go stale unnoticed. Any suppression that
// is genuinely warranted should be `#[expect]` with a reason, and warranted is a decision for the
// repository owner, not for the author of the code being suppressed.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const SKIP_DIRS = new Set(["node_modules", "dist", "target", ".git", "dist-docs"]);
const ROOTS = ["src", "scripts", "examples"];

// A pattern must contain a literal copy of what it matches, so this file necessarily writes every
// directive it bans. Unlike prose, that cannot be paraphrased away. Marked lines are skipped and
// the active count prints with every result: an uncounted exemption is a backdoor, and a silent
// one is indistinguishable from a rule that does not apply.
const EXAMPLE_EXEMPT = /\bEXAMPLE:/;

/**
 * Each entry names one way to silence "this is never used" in this repo's languages. Rust's
 * attribute forms and the TypeScript comment directives are the same act expressed twice, so they
 * are gated together rather than in separate checks that can drift apart.
 */
const SUPPRESSIONS = [
  {
    name: "rust dead_code",
    exts: [".rs"],
    // Matches item (`#[...]`) and module (`#![...]`) scope, and any position in a grouped
    // attribute such as `#[allow(clippy::x, dead_code)]`.
    re: /#!?\[\s*(?:allow|expect)\s*\([^)]*\bdead_code\b/,
  },
  {
    name: "rust unused",
    exts: [".rs"],
    // `unused` itself plus the whole family it groups: unused_variables, unused_imports,
    // unused_mut, unused_must_use and the rest all share the prefix.
    re: /#!?\[\s*(?:allow|expect)\s*\([^)]*\bunused(?:_\w+)?\b/,
  },
  {
    name: "eslint unused-vars",
    exts: [".ts", ".svelte", ".js", ".mjs"],
    re: /eslint-disable(?:-next-line|-line)?[^\n]*no-unused-vars/, // EXAMPLE:
  },
  {
    name: "typescript checker suppression",
    exts: [".ts", ".svelte"],
    // These silence every diagnostic on the following line or in the whole file, which includes
    // unused-symbol reporting; the blast radius is wider than the annotation admits.
    re: /@ts-ignore|@ts-nocheck/,
  },
];

/** Recursively collects source paths under `dir`; an absent root yields none. */
function sources(dir) {
  const out = [];
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of entries) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sources(p));
    else out.push(p);
  }
  return out;
}

const norm = (p) => p.split("\\").join("/");
const scanned = ROOTS.flatMap(sources).map(norm);
const hits = [];
let exempted = 0;

for (const path of scanned) {
  const applicable = SUPPRESSIONS.filter((s) => s.exts.some((e) => path.endsWith(e)));
  if (applicable.length === 0) continue;
  readFileSync(path, "utf8")
    .split("\n")
    .forEach((line, i) => {
      const hit = applicable.find((s) => s.re.test(line));
      if (!hit) return;
      // Counted only when the line would otherwise have been a hit, so the number means
      // "suppressions currently permitted" rather than "lines carrying a marker".
      if (EXAMPLE_EXEMPT.test(line)) {
        exempted += 1;
        return;
      }
      hits.push({ path, line: i + 1, kind: hit.name, text: line.trim() });
    });
}
const provenance = `${scanned.length} file(s) scanned${exempted ? `; ${exempted} EXAMPLE-exempt` : ""}`;

// Scanning nothing and finding nothing produce the same zero, and a broken root reads as a pass.
// Refuse the ambiguous result: an empty scan is an error, never a clean bill of health.
if (scanned.length === 0) {
  console.error(`no files found under ${ROOTS.join(", ")}. Nothing was examined.`);
  process.exit(2);
}

if (hits.length === 0) {
  console.log(`No dead-code or unused-item suppressions. ${provenance}.`);
  process.exit(0);
}

console.error(`${hits.length} dead-code/unused suppression(s). ${provenance}.`);
console.error("A suppressed diagnostic still describes an item nothing reaches.\n");
for (const h of hits) console.error(`  ${h.path}:${h.line}  [${h.kind}]  ${h.text}`);
console.error("\nRemove the item, make it reachable, or scope it to the build that uses it.");
process.exit(1);
