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

import { readFileSync, readdirSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";

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

// Query interface. It exists so no caller has to re-derive a subset by grepping this script's
// own output: an ad-hoc pattern is a fresh, unvalidated instrument every time it is written, it
// cannot be compared against a number a DIFFERENT ad-hoc pattern produced earlier, and one keyed
// on `path:line` silently goes stale the moment an edit shifts a line. Ask this script instead.
//   --scope <prefix>   restrict to paths under a prefix (repeatable); forward slashes always
//   --by-area          per-directory table instead of the per-site list
//   --json             machine-readable {total, byKind, byArea, hits}
const argv = process.argv.slice(2);
const scopes = argv.flatMap((a, i) => (a === "--scope" ? [argv[i + 1]] : [])).filter(Boolean);
const wantArea = argv.includes("--by-area");
const wantJson = argv.includes("--json");

/** Repo-relative path with forward slashes, so a scope reads the same on every platform. */
const norm = (p) => p.split("\\").join("/");
const inScope = (p) => scopes.length === 0 || scopes.some((s) => p.startsWith(norm(s)));

const scanned = sources("src").map(norm).filter(inScope);
const hits = [];
for (const path of scanned) {
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

// A scope that matches no files and a scope that is genuinely clean both produce zero hits, and
// telling them apart by eye is impossible — a mistyped prefix reads as success. Refuse to report
// the ambiguous zero: a scope matching no files is an error, never a pass.
if (scopes.length > 0 && scanned.length === 0) {
  console.error(`--scope matched 0 files: ${scopes.join(", ")}`);
  console.error("Nothing was examined, so this is not a clean result. Check the prefix.");
  process.exit(2);
}

// A bare count carries no record of the instrument that produced it, so a widened pattern and a
// regressed codebase are the same number going up, and a broken scanner and a clean scope are the
// same zero. Every count this script prints is therefore stamped with a fingerprint of the rules
// that produced it, and a run whose fingerprint differs from the previous run for the same scope
// says so instead of inviting a comparison that is not valid.
const INSTRUMENT = createHash("sha256")
  .update(
    JSON.stringify([
      BANNED.map((b) => [b.name, b.re.source, b.re.flags]),
      [COMMENT.source, STRING_LITERAL.source, EXPLANATORY_STRING.source],
      EXTS,
      [...SKIP_DIRS].sort(),
    ]),
  )
  .digest("hex")
  .slice(0, 8);

const STATE_PATH = ".superpowers/rule16-instrument.json";
const scopeKey = scopes.length > 0 ? scopes.map(norm).sort().join(",") : "<repo>";

/** Prior run for this scope, or null. Absent/corrupt state is simply no prior run. */
function priorRun() {
  try {
    return JSON.parse(readFileSync(STATE_PATH, "utf8"))[scopeKey] ?? null;
  } catch {
    return null;
  }
}

/** Records this run so the NEXT one can tell an instrument change from a code change. */
function recordRun(total) {
  try {
    let all = {};
    try {
      all = JSON.parse(readFileSync(STATE_PATH, "utf8"));
    } catch {
      /* first run, or unreadable state — start fresh */
    }
    all[scopeKey] = { instrument: INSTRUMENT, total, filesScanned: scanned.length };
    mkdirSync(".superpowers", { recursive: true });
    writeFileSync(STATE_PATH, JSON.stringify(all, null, 2));
  } catch {
    /* state is an aid, never a gate: a read-only checkout must still be able to run this */
  }
}

/** The line that makes a count self-describing. Print it beside every total. */
function provenance(total) {
  const prior = priorRun();
  const head = `instrument ${INSTRUMENT}; ${scanned.length} file(s) scanned`;
  if (!prior) return `${head}; no prior run recorded for this scope`;
  if (prior.instrument !== INSTRUMENT) {
    return (
      `${head}\nINSTRUMENT CHANGED since the last run for this scope ` +
      `(${prior.instrument} -> ${INSTRUMENT}). The previous total of ${prior.total} was measured ` +
      `by different rules and is NOT comparable to ${total}. Any movement here is the ruler, ` +
      `not necessarily the code.`
    );
  }
  const delta = total - prior.total;
  const dir = delta === 0 ? "unchanged" : delta > 0 ? `+${delta}` : `${delta}`;
  return `${head}; same instrument as last run: ${prior.total} -> ${total} (${dir})`;
}

/** Groups by the first three path segments (`src/server/src`, `src/modules/panels`). */
const tally = (keyOf) => {
  const m = new Map();
  for (const h of hits) m.set(keyOf(h), (m.get(keyOf(h)) ?? 0) + 1);
  return [...m].sort((a, b) => b[1] - a[1]);
};
const byKind = tally((h) => h.kind);
const byArea = tally((h) => h.path.split("/").slice(0, 3).join("/"));
const files = new Set(hits.map((h) => h.path)).size;

if (wantJson) {
  const prior = priorRun();
  console.log(
    JSON.stringify(
      {
        total: hits.length,
        instrument: INSTRUMENT,
        comparableToPrior: prior ? prior.instrument === INSTRUMENT : null,
        prior,
        filesScanned: scanned.length,
        filesWithHits: files,
        byKind,
        byArea,
        hits,
      },
      null,
      2,
    ),
  );
  recordRun(hits.length);
  process.exit(hits.length > 0 ? 1 : 0);
}

if (wantArea) {
  const scopeNote = scopes.length > 0 ? ` under ${scopes.join(", ")}` : "";
  console.log(`${hits.length} site(s) in ${files} file(s)${scopeNote}`);
  console.log(provenance(hits.length));
  for (const [area, n] of byArea) console.log(`${String(n).padStart(5)}  ${area}`);
  recordRun(hits.length);
  process.exit(hits.length > 0 ? 1 : 0);
}

if (hits.length > 0) {
  console.error(`RULE 16: ${hits.length} ephemeral reference(s) in code comments.`);
  console.error("A code comment must be durable commentary about the code and only the code.");
  console.error(provenance(hits.length));
  console.error("Fix by stating the CONSTRAINT inline and dropping the POINTER.\n");
  for (const [kind, n] of byKind) console.error(`  ${String(n).padStart(4)}  ${kind}`);
  console.error("");
  for (const [area, n] of byArea) console.error(`  ${String(n).padStart(4)}  ${area}`);
  console.error("");
  for (const h of hits) console.error(`  ${h.path}:${h.line}  [${h.kind}]  ${h.text}`);
  recordRun(hits.length);
  process.exit(1);
}

console.log(`RULE 16: no ephemeral references in code comments.`);
console.log(provenance(0));
recordRun(0);
