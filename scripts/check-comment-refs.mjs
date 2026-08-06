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
  { name: "milestone/task id", re: /\bM\d+[a-z]?-\d+\b|\bM\d+[a-z]\b/ },
  {
    name: "repo document pointer",
    re: /docs\/[\w./-]+\.md|\b(?:TODO|OPEN_BUGS|CLOSED_BUGS|POST_WORK_FINDINGS|ARCHITECTURE|PLAN)\.md|ARCHITECTURE\s*§/,
  },
  { name: "dated plan/spec file", re: /\b20\d\d-\d\d-\d\d[\w-]*\.md/ },
  { name: "sweep / round name", re: /\b[Ss]weep \d+|\bfix round \d+/ },
  { name: "process marker", re: /POST_WORK:/ },
];

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
    if (!COMMENT.test(line)) return;
    const hit = BANNED.find((b) => b.re.test(line));
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
