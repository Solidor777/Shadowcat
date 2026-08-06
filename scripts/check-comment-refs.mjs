// Ratchet: fails when code comments gain NEW references to ephemeral, process-assigned things.
//
// Rule enforced: `docs/design/doc-sweep-truthfulness-rules.md` RULE 16 — a code comment is durable
// commentary about the code and only the code. Milestone ids, repo-document pointers, dated spec
// files, sweep names and process markers all name something whose identity a process assigns; when
// it is renumbered, closed or superseded the comment points at nothing, and — unlike a stale claim
// about code — no reader and no tool can detect it. This script is that missing detector.
//
// Hard gate: any ephemeral reference in a code comment fails, legacy included. A grandfathered
// site is indistinguishable from a new one to every future reader, so exempting the backlog would
// preserve exactly the defect the rule exists to remove.
//
// Implicit coupling: the patterns below mirror RULE 16's banned table. Adding a category there
// without adding it here leaves the rule unenforced, which is how the original 266 accumulated.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const SKIP_DIRS = new Set(["node_modules", "dist", "target", ".git", "dist-docs"]);
const EXTS = [".ts", ".rs", ".svelte"];

/** A line whose content is a comment. Block-comment bodies are matched via the leading `*`. */
const COMMENT = /^\s*(\/\/|\*|\/\/\/|\/\/!)/;

const BANNED = [
  // `M8`, `M8c`, `M8c-1` are one id shape: the bare form carries no less process identity than
  // the suffixed one, and a pattern requiring the suffix reads a plain `M8` as clean.
  { name: "milestone/task id", re: /\bM\d+[a-z]?(?:-\d+)?\b/ },
  // Phase checkpoints (`D9`), workstreams (`W1`) and numbered invariants (`I4`) are ids a
  // process assigns, resolvable only by a reader who has the process artifact.
  { name: "phase / workstream / invariant id", re: /\b[DIW]\d+\b/ },
  {
    name: "repo document pointer",
    re: /docs\/[\w./-]+\.md|\b(?:TODO|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS|ARCHITECTURE|PLAN)\.md|ARCHITECTURE\s*[§#]|\binvariant\s*#?\s*\d+/i,
  },
  { name: "dated plan/spec file", re: /\b20\d\d-\d\d-\d\d[\w-]*\.md/ },
  // A date stamps a comment with when someone wrote it, which is not behaviour. Bare dates
  // inside example data (`backups/2026-07-30`) are program illustration, so a match requires a
  // parenthesised or "as of" form.
  {
    name: "date stamp",
    re: /\(\s*20\d\d-\d\d-\d\d\s*\)|\bas of \d|\bas of (?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)/i,
  },
  // Prose describing a superseded state of the code. Only high-precision forms match: "no
  // longer" overwhelmingly describes runtime data ("an id that no longer names a scene"), and
  // flagging it would train writers to dodge the word rather than drop the narration. The
  // general prohibition is a review rule, not a pattern.
  {
    name: "history narration",
    re: /\bpreviously\b|\bformerly\b|\bhistorically\b|\b(?:before|after) the (?:fix|refactor|change|rewrite)\b/i,
  },
  // An unnamed "the spec" is the same defect as a named one and is strictly worse to resolve:
  // the reader cannot even tell which document went stale.
  // Matches a reference to a spec DOCUMENT, not the word: `spec` is also a parameter name
  // (`setBackground(spec)`) and the e2e test-file suffix, and neither points outside the code.
  {
    name: "unnamed spec reference",
    re: /\bspec\s*§|\b(?:the|this|design|parent|wire|per)\s+spec\b|\bspec'?d\b|\bspec\s*:/i,
  },
  {
    name: "sweep / round / review marker",
    re: /\b[Ss]weep \d+|\bfix[- ]round|\bbuddy-check|\bfinding \d+/i,
  },
  { name: "process marker", re: /POST_WORK:/ },
];

// RULE 16 extends to code-facing string literals (assert! messages, test names): a developer
// reads an assertion message at failure time exactly as they read a comment, and it goes stale
// the same undetectable way. Ruled in scope by the user.
const STRING_LITERAL = /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g;

// Only strings a developer READS as explanation qualify — assertion/panic messages and test
// names. A string that is program data (a fixture's world name, a document key) names something
// inside the program, so an id-shaped collision there is not a reference to anything.
const EXPLANATORY_STRING =
  /\bassert(?:_eq|_ne)?!|\bpanic!|\.expect\(|\bexpect\(|^\s*(?:async\s+)?(?:test|it|describe)\s*(?:\.\w+)?\s*\(/;

/** Recursively collects source paths under `dir`. */
function sources(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...sources(p));
    else if (EXTS.some((e) => name.endsWith(e))) out.push(p);
  }
  return out;
}

const hits = [];
for (const path of sources("src")) {
  const lines = readFileSync(path, "utf8").split("\n");
  lines.forEach((line, i) => {
    // A comment line is checked whole; a code line is checked only inside its string literals,
    // so identifiers and paths that are part of the program are never flagged.
    const subject = COMMENT.test(line)
      ? line
      : EXPLANATORY_STRING.test(line)
        ? (line.match(STRING_LITERAL) ?? []).join(" ")
        : "";
    if (subject === "") return;
    const hit = BANNED.find((b) => b.re.test(subject));
    if (hit) hits.push({ path, line: i + 1, kind: hit.name, text: line.trim() });
  });
}

if (hits.length > 0) {
  console.error(`RULE 16: ${hits.length} ephemeral reference(s) in code comments.`);
  console.error("A code comment must be durable commentary about the code and only the code.");
  console.error("Fix by stating the CONSTRAINT inline and dropping the POINTER.\n");
  const byKind = new Map();
  for (const h of hits) byKind.set(h.kind, (byKind.get(h.kind) ?? 0) + 1);
  for (const [kind, n] of [...byKind].sort((a, b) => b[1] - a[1])) {
    console.error(`  ${String(n).padStart(4)}  ${kind}`);
  }
  console.error("");
  for (const h of hits) console.error(`  ${h.path}:${h.line}  [${h.kind}]  ${h.text}`);
  process.exit(1);
}

console.log("RULE 16: no ephemeral references in code comments.");
