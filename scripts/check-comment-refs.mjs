// Fails when a code comment references an ephemeral, process-assigned thing.
//
// A code comment is durable commentary about the code and only the code. Milestone ids,
// repo-document pointers, dated spec files, sweep names and process markers all name something
// whose identity a process assigns; when it is renumbered, closed or superseded the comment points
// at nothing, and — unlike a stale claim about code — no reader and no tool can detect it. That
// undetectability is the whole defect, and this script is the missing detector.
//
// Hard gate: every occurrence fails, legacy included. A grandfathered site is indistinguishable
// from a new one to every future reader, so exempting a backlog preserves exactly the defect being
// removed.
//
// This file is itself in scope (see ROOTS) and names no document, rule number or workspace, for
// the reason it enforces: a checker that cites the numbered rule it implements breaks silently
// when that rule is renumbered, while still reporting success. The property is the durable name.

import { readFileSync, readdirSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";

const SKIP_DIRS = new Set(["node_modules", "dist", "target", ".git", "dist-docs"]);
const EXTS = [".ts", ".rs", ".svelte", ".mjs", ".js"];

// The checkers are in scope with the code they check. A gate that excludes its own directory
// guarantees exactly one blind spot, and it is the blind spot where a violation does the most
// damage: an enforcement script that cites an ephemeral document teaches every reader that the
// citation is acceptable, and no run will ever say otherwise.
const ROOTS = ["src", "scripts"];

// Repo-root config files are code too — an eslint config decides what ships. They are collected by
// walking the root non-recursively rather than by listing them, so a new config is in scope the
// day it is added instead of the day someone remembers to enumerate it.
const rootFiles = () =>
  readdirSync(".").filter((n) => EXTS.some((e) => n.endsWith(e)) && statSync(n).isFile());

// EXAMPLE: A detector must be able to quote the shape it detects. `M8` in a pattern's docs is not
// a reference to a milestone — it describes this code's own matching behaviour, and stays true
// whether or not a milestone was ever numbered 8. Marked lines are exempt, and the
// count of active exemptions is printed with every result: an exemption nobody counts is a
// backdoor, and a silent one is indistinguishable from a rule that does not apply.
const EXAMPLE_EXEMPT = /\bEXAMPLE:/;

/** A line whose content is a comment. Block-comment bodies are matched via the leading `*`. */
const COMMENT = /^\s*(\/\/|\*|\/\/\/|\/\/!)/;

const BANNED = [
  // EXAMPLE: `M8`, `M8c`, `M8c-1` are one id shape. The bare form carries no less process
  // EXAMPLE: identity than the suffixed one, and a pattern requiring the suffix reads `M8` clean.
  { name: "milestone/task id", re: /\bM\d+[a-z]?(?:-\d+)?\b/ },
  // EXAMPLE: Phase checkpoints (`D9`), workstreams (`W1`) and numbered invariants (`I4`) are ids
  // a process assigns, resolvable only by a reader who holds the process artifact.
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
  // EXAMPLE: An unnamed "the spec" is the same defect as a named one and strictly worse to
  // resolve: the reader cannot even tell which document went stale. Matches a reference to a spec
  // DOCUMENT, not the word — `spec` is also a parameter name (`setBackground(spec)`) and the e2e
  // test-file suffix, and neither points outside the code.
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

// The rule extends to code-facing string literals (assert! messages, test names): a developer
// reads an assertion message at failure time exactly as they read a comment, and it goes stale
// the same undetectable way. Ruled in scope by the user.
const STRING_LITERAL = /"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g;

// The in-scope/out-of-scope line is drawn by the STRING'S OWN SHAPE, not by the syntax around it.
// A literal containing whitespace is prose some human reads; a whitespace-free literal is a token
// the program acts on (a fixture's world name, a document key, an id), where an id-shaped
// collision is not a reference to anything.
//
// Shape, not context, because context is a whitelist and prose escapes through whatever the list
// EXAMPLE: `veto("no top dock zone exists (spec D4)")` reaches a developer at runtime exactly as an
// assertion message does, but sits in no assert/test call. Whitelisting contexts means enumerating
// every way a human-readable string can surface, and being silently wrong each time one is missed.
const PROSE_LITERAL = /\s/;

// Explanatory contexts stay in scope on top of the shape test, since a test name or assertion
// EXAMPLE: message can legitimately be a single token — `it("M13-0")` is prose minus the spaces.
const EXPLANATORY_STRING =
  /\bassert(?:_eq|_ne)?!|\bpanic!|\.expect\(|\bexpect\(|^\s*(?:async\s+)?(?:test|it|describe)\s*(?:\.\w+)?\s*\(/;

/** Recursively collects source paths under `dir`; called once per entry in ROOTS. */
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
//   --cover <prefix>   verify prefixes partition the tree disjointly and exhaustively (repeatable)
const argv = process.argv.slice(2);
const scopes = argv.flatMap((a, i) => (a === "--scope" ? [argv[i + 1]] : [])).filter(Boolean);
const wantArea = argv.includes("--by-area");
const wantJson = argv.includes("--json");
const covers = argv.flatMap((a, i) => (a === "--cover" ? [argv[i + 1]] : [])).filter(Boolean);

/** Repo-relative path with forward slashes, so a scope reads the same on every platform. */
const norm = (p) => p.split("\\").join("/");
const inScope = (p) => scopes.length === 0 || scopes.some((s) => p.startsWith(norm(s)));

const scanned = [...ROOTS.flatMap(sources), ...rootFiles()].map(norm).filter(inScope);
const hits = [];
let exempted = 0;
for (const path of scanned) {
  const lines = readFileSync(path, "utf8").split("\n");
  lines.forEach((line, i) => {
    if (EXAMPLE_EXEMPT.test(line)) {
      exempted += 1;
      return;
    }
    // A comment line is checked whole; a code line is checked only inside its string literals,
    // so identifiers and paths that are part of the program are never flagged. Of those literals,
    // prose ones always count and token-shaped ones count only in an explanatory context.
    const literals = COMMENT.test(line) ? [] : (line.match(STRING_LITERAL) ?? []);
    const explanatory = EXPLANATORY_STRING.test(line);
    const subject = COMMENT.test(line)
      ? line
      : literals.filter((l) => explanatory || PROSE_LITERAL.test(l.slice(1, -1))).join(" ");
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
      [
        COMMENT.source,
        STRING_LITERAL.source,
        EXPLANATORY_STRING.source,
        PROSE_LITERAL.source,
        EXAMPLE_EXEMPT.source,
      ],
      ROOTS,
      EXTS,
      [...SKIP_DIRS].sort(),
    ]),
  )
  .digest("hex")
  .slice(0, 8);

// A run memo, so the next run can distinguish a changed ruler from changed code. It lives in the
// conventional JS tool cache rather than any workspace a process creates: a durable script that
// hardcodes a scaffolding directory stops working when that scaffolding is renamed or cleaned, and
// the failure is a silently skipped comparison, not an error.
const STATE_DIR = "node_modules/.cache/comment-refs";
const STATE_PATH = `${STATE_DIR}/instrument.json`;
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
    mkdirSync(STATE_DIR, { recursive: true });
    writeFileSync(STATE_PATH, JSON.stringify(all, null, 2));
  } catch {
    /* state is an aid, never a gate: a read-only checkout must still be able to run this */
  }
}

/** The line that makes a count self-describing. Print it beside every total. */
function provenance(total) {
  const prior = priorRun();
  const ex = exempted > 0 ? `; ${exempted} line(s) EXAMPLE-exempt` : "";
  const head = `instrument ${INSTRUMENT}; ${scanned.length} file(s) scanned${ex}`;
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

// --cover: do these prefixes partition the tree, disjointly and exhaustively?
//
// Whenever a surface is divided for parallel work, the division is the least-reviewed artifact
// involved: written once, read by each worker only for its own slice, never checked whole. Its
// failure modes are asymmetric. An overlap duplicates effort loudly enough to notice; a file no
// slice claims is simply never examined, while every worker still reports a clean scope and the
// totals still look plausible. Unmeasured coverage is indistinguishable from complete coverage.
//
// This asks only about durable things — path prefixes and the files actually on disk. It has no
// notion of a task, an owner, a campaign or a plan: those expire, and durable tooling that models
// them rots in step with them while continuing to report success. The prefix-to-worker mapping
// belongs in the ephemeral brief that assigns it; the question of whether a set of prefixes covers
// the tree is a property of the tree alone, and is answered here.
//
// Fails on: a prefix matching no file (a typo is indistinguishable from a legitimately empty
// slice), a file claimed by two prefixes, and any scanned file no prefix claims.
if (covers.length > 0) {
  if (scopes.length > 0) {
    console.error("--cover measures the whole tree; do not combine it with --scope.");
    process.exit(2);
  }
  const owner = new Map();
  const problems = [];

  for (const prefix of covers) {
    const matched = scanned.filter((p) => p.startsWith(norm(prefix)));
    if (matched.length === 0) {
      problems.push(`EMPTY    "${prefix}" matches no file. A typo reads as an empty slice.`);
    }
    for (const p of matched) {
      if (owner.has(p)) problems.push(`OVERLAP  ${p} claimed by "${owner.get(p)}" and "${prefix}".`);
      else owner.set(p, prefix);
    }
  }

  const unclaimed = scanned.filter((p) => !owner.has(p));
  if (unclaimed.length > 0) {
    const n = hits.filter((h) => !owner.has(h.path)).length;
    problems.push(`GAP      ${unclaimed.length} file(s) under no prefix, holding ${n} site(s):`);
    for (const p of unclaimed.slice(0, 20)) {
      const c = hits.filter((h) => h.path === p).length;
      problems.push(`           ${p}${c ? `  (${c} site(s))` : ""}`);
    }
    if (unclaimed.length > 20) problems.push(`           … ${unclaimed.length - 20} more`);
  }

  console.log(provenance(hits.length));
  console.log("");
  for (const prefix of covers) {
    const n = hits.filter((h) => owner.get(h.path) === prefix).length;
    const f = scanned.filter((p) => owner.get(p) === prefix).length;
    console.log(`  ${String(n).padStart(4)} site(s)  ${String(f).padStart(4)} file(s)  ${prefix}`);
  }
  console.log("");
  if (problems.length === 0) {
    console.log(`OK: ${covers.length} prefix(es) cover all ${scanned.length} file(s) exactly once.`);
    console.log(`Sites accounted for: ${hits.length}.`);
    process.exit(0);
  }
  for (const p of problems) console.error(p);
  console.error(`\n${problems.length} structural problem(s). This split is not safe to dispatch.`);
  process.exit(2);
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
  console.error(`${hits.length} ephemeral reference(s) in code comments.`);
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

console.log(`No ephemeral references in code comments.`);
console.log(provenance(0));
recordRun(0);
