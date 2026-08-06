// Fails if a dispatch brief encodes a SUPERSEDED version of the doc-sweep rule set.
//
// Constraint this enforces: a brief is authored once and dispatched later, so it
// freezes whatever the rules said at authoring time. When a rule is ratified or
// replaced, every unsent brief silently keeps mandating the old one — and the
// implementer obeys the brief, not the rules file. Three sweep-13 briefs shipped
// mandating Rule 13's `file:line` citations after RULE 15 replaced it.
//
// Implicit coupling: the banned patterns below mirror `docs/design/doc-sweep-truthfulness-rules.md`.
// Adding or superseding a rule there REQUIRES adding its stale form here, or this
// check goes quiet exactly when it is needed.
//
// Scope: dispatch briefs only (`*-brief.md`). Reports, ledgers and diffs are
// historical records of what was true when written — rewriting those would
// falsify the record, which is the defect this check exists to prevent.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

/** Superseded rule encodings. Each entry fails a brief that still mandates it. */
const BANNED = [
  {
    pattern: /Path-qualify every citation/i,
    why: "Rule 13 (path-qualified `file:line`) is superseded by RULE 15 (cite symbols).",
  },
  {
    pattern: /all 1[0-5] rules/i,
    why: "The rule set is RULE 0 plus 16 rules; a lower count means the brief predates RULE 16.",
  },
];

// A brief that already governed executed work is a historical record: it states what the
// implementer was actually told. Rewriting it would falsify that record — the same defect
// RULE 16 exists to prevent — so an executed brief is FROZEN with this marker and skipped.
// Only unexecuted briefs must track the current rules, because only they still bind anyone.
const FROZEN = /^<!--\s*frozen:/m;

/** Recursively collects `*-brief.md` paths under `dir`. */
function briefs(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...briefs(p));
    else if (name.endsWith("-brief.md")) out.push(p);
  }
  return out;
}

const root = ".superpowers/sdd";
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
      console.error(`${path}:${line + 1}\n  stale rule encoding: ${why}`);
      failed++;
    }
  }
}

if (failed > 0) {
  console.error(`\n${failed} stale rule encoding(s) in dispatch briefs.`);
  process.exit(1);
}
console.log(
  `${briefs(root).length - frozen} live dispatch brief(s) carry no superseded rule encoding ` +
    `(${frozen} frozen as executed).`,
);
