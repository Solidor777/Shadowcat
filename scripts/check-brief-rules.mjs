// EXAMPLE: Fails if a dispatch brief mandates an instruction that has since been superseded.
//
// Constraint this enforces: a brief is authored once and dispatched later, so it freezes whatever
// the guidance said at authoring time. When guidance is replaced, every unsent brief silently
// keeps mandating the old form — and an implementer obeys the brief, not the guidance. A brief is
// also the least-reviewed artifact in a dispatch: its author reads it once and each implementer
// reads only its own slice, so nothing examines it as a whole after it is written.
//
// Implicit coupling: each entry below must be added when guidance is superseded, or this check
// goes quiet exactly when it is needed. Entries state the superseded INSTRUCTION, never the
// identifier of whatever numbered rule replaced it: that numbering is reassigned by a process, so
// a check keyed on it silently stops matching while still reporting success.
//
// Scope: dispatch briefs only (`*-brief.md`). Reports, ledgers and diffs are historical records of
// what was true when written — rewriting those would falsify the record, which is the defect this
// check exists to prevent.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/** Superseded instructions. Each entry fails a brief that still mandates one. */
const BANNED = [
  {
    pattern: /Path-qualify every citation/i,
    why: "Superseded: cite a SYMBOL, never a file path or line number. A line number is invalidated by any edit above it, and nothing detects that.",
  },
  {
    // Any count, not a range of known-stale ones: a pattern encoding which counts are currently
    // wrong asserts what the current count is, so it must be widened every time the set grows and
    // goes quiet — still passing — when it is not.
    pattern: /all \d+ rules\b/i,
    why: "A brief must not state how many rules exist. Every count expires as rules are added, and a brief carrying a stale one instructs an implementer to apply a subset and report success.",
  },
];

// A brief that already governed executed work is a historical record: it states what the
// implementer was actually told. Rewriting it would falsify that record, so an executed brief
// carries this marker and is skipped. Only an unexecuted brief must track current guidance,
// because only an unexecuted brief still binds anyone.
const FROZEN = /^<!--\s*frozen:/m;

/** Recursively collects `*-brief.md` paths under `dir`; an absent directory yields none. */
function briefs(dir) {
  const out = [];
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of entries) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...briefs(p));
    else if (name.endsWith("-brief.md")) out.push(p);
  }
  return out;
}

// The caller names the directory. Hardcoding one would bind this script to a workspace layout a
// process invents and later renames, and the failure mode is a scan of nothing reported as a pass.
const root = process.argv[2];
if (!root) {
  console.error("usage: check-brief-rules.mjs <brief-directory>");
  process.exit(2);
}
let failed = 0;
let frozen = 0;
for (const path of briefs(root)) {
  const text = readFileSync(path, "utf8");
  if (FROZEN.test(text)) {
    frozen++;
    continue;
  }
  for (const { pattern, why } of BANNED) {
    const line = text.split("\n").findIndex((l) => pattern.test(l));
    if (line >= 0) {
      console.error(`${path}:${line + 1}\n  superseded instruction: ${why}`);
      failed++;
    }
  }
}

if (failed > 0) {
  console.error(`\n${failed} superseded instruction(s) in dispatch briefs.`);
  process.exit(1);
}

// A directory holding no briefs and a directory of clean briefs both print zero failures, and a
// mistyped path reads as a pass. Refuse the ambiguous zero: examining nothing is never a result.
const total = briefs(root).length;
if (total === 0) {
  console.error(`no *-brief.md found under "${root}". Nothing was examined; check the path.`);
  process.exit(2);
}
// This tool's subject IS dispatch briefs, so its own report must name them.
console.log(
  `${total - frozen} live dispatch brief(s) carry no superseded instruction ` + // EXAMPLE:
    `(${frozen} frozen as executed).`,
);
