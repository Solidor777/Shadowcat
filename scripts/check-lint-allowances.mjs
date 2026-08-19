// Fails when code suppresses a lint diagnostic without a recorded, per-instance approval.
//
// A suppression silences a diagnostic without changing what the diagnostic describes. The item is
// still unreached, the signature still takes too many arguments, the value is still `any` — the
// compiler simply stops saying so, and the annotation then reads as a considered design decision
// to every later reader, which is precisely what it is not unless someone decided it.
//
// A module-scoped suppression is strictly worse than an item-scoped one: it silences every FUTURE
// occurrence in that file too, so real violations accumulate where no build will ever report them.
//
// A suppression also cannot tell you it has expired. Auditing this repo's eight dead-code
// allowances found three that suppressed nothing at all — the items had since acquired real
// callers and the annotation stayed behind claiming otherwise.
//
// `#[expect(...)]` is NOT an accepted alternative and is matched here alongside `#[allow(...)]`.
// It self-invalidates when the lint stops firing, which makes it the better ANNOTATION and is
// exactly why it is tempting — but the rule's intent is that the suppressed condition not exist,
// not that suppressions be kept fresh. Fix the code: make the item reachable, delete it, scope it
// to the build that uses it, or refactor the signature.
//
// Approval is per instance and belongs to the repository owner, never to the author of the code
// being suppressed. Approved instances live in the allowlist named below.

import { readFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join } from "node:path";
// An attribute quoted inside a doc comment is prose, not a suppression. Telling code from comment
// is shared with the ephemeral-reference gate rather than reimplemented here: two scanners with
// their own idea of what a comment is disagree about the same file, and neither can say which is
// right.
import { splitLine } from "./lib/comment-span.mjs";

const SKIP_DIRS = new Set(["node_modules", "dist", "target", ".git", "dist-docs"]);
const ROOTS = ["src", "scripts", "examples"];
const ALLOWLIST = ".claude/suppression-allowlist.toml";

// Repo-root config files that can lower a lint from a distance. Listed because they are a fixed,
// small set at a fixed location; a lowering here suppresses the same diagnostic as an inline
// annotation while leaving no annotation for the scan above to find.
const MANIFESTS = ["Cargo.toml", "clippy.toml", "eslint.config.js"];

// A pattern must contain a literal copy of what it matches, so `SUPPRESSIONS`' own regex literals
// necessarily reproduce every directive they ban. Marked lines are skipped and the active count
// prints with every result: an uncounted exemption is a backdoor, and a silent one is
// indistinguishable from a rule that does not apply.
const EXAMPLE_EXEMPT = /\bEXAMPLE:/;

/**
 * Each entry names one way to silence a diagnostic in this repo's languages, and extracts the
 * SPECIFIC lint names on a line — the allowlist keys on the individual lint, not the family, so
 * approving one lint on an item never silently approves another added beside it later.
 */
const SUPPRESSIONS = [
  {
    name: "rust",
    exts: [".rs"],
    // Item (`#[...]`) and module (`#![...]`) scope, `allow` and `expect`, and every lint inside a
    // grouped attribute such as `#[allow(clippy::x, dead_code)]`.
    re: /#!?\[\s*(?:allow|expect)\s*\(([^)]*)\)/,
    extract: (m) =>
      m[1]
        .split(",")
        .map((s) => s.trim().replace(/\s*=.*$/, ""))
        .filter((s) => /^[a-z][a-z0-9_]*(::[a-z][a-z0-9_]*)*$/.test(s)),
  },
  {
    name: "eslint",
    exts: [".ts", ".svelte", ".js", ".mjs"],
    re: /eslint-disable(?:-next-line|-line)?\s+([^\n*]+)/, // EXAMPLE:
    extract: (m) =>
      m[1]
        .split(",")
        .map((s) => s.trim().replace(/\s*--.*$/, ""))
        .filter((s) => /^[@a-z][\w@/-]*$/i.test(s)),
  },
  {
    name: "typescript",
    exts: [".ts", ".svelte"],
    // These silence every diagnostic on the following line or in the whole file, which is a wider
    // blast radius than the annotation admits.
    re: /(@ts-ignore|@ts-nocheck)/,
    extract: (m) => [m[1]],
  },
];

/** A declaration this scanner will name as the item a suppression sits on. */
const DECL = [
  /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?(?:fn|struct|enum|trait|mod|type|union)\s+([A-Za-z_][\w]*)/,
  /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+(?:mut\s+)?([A-Za-z_][\w]*)/,
  /^\s*impl(?:\s*<[^>]*>)?\s+(?:[\w:]+\s+for\s+)?([A-Za-z_][\w]*)/,
  /^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?(?:function|class|interface|type|const|let|var)\s+([A-Za-z_$][\w$]*)/,
  /^\s*(?:public|private|protected|readonly|static|get|set|#)*\s*([A-Za-z_$#][\w$]*)\s*[(:=]/,
];

const declName = (line) => {
  if (/^\s*(?:\/\/|\/\*|\*|#!?\[|$)/.test(line)) return null;
  for (const re of DECL) {
    const m = line.match(re);
    if (m) return m[1];
  }
  return null;
};

/**
 * The symbol a suppression governs, used as its allowlist key instead of a line number: a
 * positional key rots on the next edit and nothing fails when it does, whereas moving an item
 * within a file keeps its entry and renaming or deleting it correctly invalidates one.
 *
 * A module-scoped `#![...]` governs the file itself. Otherwise the item is the next declaration
 * below (the Rust attribute and `eslint-disable-next-line` forms both attach downward); when the
 * suppressed line is not itself a declaration, the nearest enclosing one above is used.
 */
function itemOf(lines, i) {
  if (/#!\s*\[/.test(lines[i])) return "<module>";
  for (let j = i + 1; j < Math.min(lines.length, i + 12); j += 1) {
    const n = declName(lines[j]);
    if (n) return n;
    if (lines[j].trim() && !/^\s*(?:#\[|\/\/|\/\*|\*)/.test(lines[j])) break;
  }
  for (let j = i - 1; j >= 0; j -= 1) {
    const n = declName(lines[j]);
    if (n) return n;
  }
  return "<file>";
}

/**
 * Reads the allowlist. Deliberately accepts only `[[allow]]` tables of double-quoted string
 * values and ERRORS on anything else rather than skipping it: a line this parser silently ignored
 * would be an approval nobody granted or a stale entry nobody caught.
 */
function loadAllowlist() {
  if (!existsSync(ALLOWLIST)) return [];
  const out = [];
  let cur = null;
  readFileSync(ALLOWLIST, "utf8")
    .split("\n")
    .forEach((raw, i) => {
      const line = raw.replace(/\r$/, "").trim();
      if (line === "" || line.startsWith("#")) return;
      if (line === "[[allow]]") {
        cur = {};
        out.push(cur);
        return;
      }
      const m = line.match(/^([a-z_]+)\s*=\s*"((?:[^"\\]|\\.)*)"$/);
      if (!m || cur === null) {
        console.error(`${ALLOWLIST}:${i + 1}: cannot parse. Expected [[allow]] or key = "value".`);
        console.error(`  got: ${raw}`);
        process.exit(2);
      }
      cur[m[1]] = m[2].replace(/\\"/g, '"');
    });
  return out;
}

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
  const lines = readFileSync(path, "utf8").split("\n");
  let lexState = { inBlock: false, inHtml: false };
  lines.forEach((line, i) => {
    const split = splitLine(line, lexState);
    lexState = split.state;
    for (const s of applicable) {
      // Rust attributes are code, so only the code span can carry one. The comment directives are
      // the opposite — they live in comment text by construction — so they are matched on the
      // whole line.
      const subject = s.name === "rust" ? split.code : line;
      const m = subject.match(s.re);
      if (!m) continue;
      // Counted only when the line would otherwise have been a hit, so the number means
      // "suppressions currently permitted" rather than "lines carrying a marker".
      if (EXAMPLE_EXEMPT.test(line)) {
        exempted += 1;
        return;
      }
      for (const lint of s.extract(m)) {
        hits.push({ file: path, item: itemOf(lines, i), lint, line: i + 1, text: line.trim() });
      }
    }
  });
}

// A lint lowered in a manifest suppresses the same diagnostic from further away, leaving no
// annotation for the scan above to find. Without this check it is the one surviving route around
// the allowlist.
const lowered = [];
for (const f of MANIFESTS) {
  if (!existsSync(f)) continue;
  if (f === "clippy.toml") {
    lowered.push({ file: f, why: "a clippy.toml configures lint thresholds repo-wide" });
    continue;
  }
  readFileSync(f, "utf8")
    .split("\n")
    .forEach((raw, i) => {
      const line = raw.trim();
      if (/^\[lints(\.\w+)?\]/.test(line)) {
        lowered.push({ file: f, line: i + 1, why: `manifest lint table: ${line}` });
      }
      if (/^"?[@\w/-]+"?\s*:\s*(?:"off"|0)\s*,?$/.test(line)) {
        lowered.push({ file: f, line: i + 1, why: `rule disabled: ${line}` });
      }
    });
}

const allow = loadAllowlist();
const key = (o) => `${o.file}|${o.item}|${o.lint}`;
const listed = new Map(allow.map((a) => [key(a), a]));
const live = new Set(hits.map(key));

const unlisted = hits.filter((h) => !listed.has(key(h)));
const stale = allow.filter((a) => !live.has(key(a)));
const unreasoned = allow.filter((a) => !(a.reason ?? "").trim());
const malformed = allow.filter((a) => !a.file || !a.item || !a.lint);

const provenance = `${scanned.length} file(s) scanned, ${allow.length} allowlisted${
  exempted ? `, ${exempted} EXAMPLE-exempt` : ""
}`;

// Scanning nothing and finding nothing produce the same zero, and a broken root reads as a pass.
// Refuse the ambiguous result: an empty scan is an error, never a clean bill of health.
if (scanned.length === 0) {
  console.error(`no files found under ${ROOTS.join(", ")}. Nothing was examined.`);
  process.exit(2);
}

// Emits the live sites as allowlist entries, reason left blank on purpose: the reason is the whole
// mechanism, and a generated one would be boilerplate that reports green forever.
if (process.argv.includes("--report")) {
  for (const h of [...hits].sort((a, b) => key(a).localeCompare(key(b)))) {
    console.log(`[[allow]]\nfile = "${h.file}"\nitem = "${h.item}"\nlint = "${h.lint}"\nreason = ""\n`);
  }
  process.exit(0);
}

const problems = unlisted.length + stale.length + unreasoned.length + malformed.length + lowered.length;
if (problems === 0) {
  console.log(`No unapproved lint suppressions. ${provenance}.`);
  process.exit(0);
}

console.error(`${problems} suppression problem(s). ${provenance}.\n`);
if (unlisted.length) {
  console.error(`NOT APPROVED (${unlisted.length}) — every suppression needs a per-instance entry:`);
  for (const h of unlisted) console.error(`  ${h.file}:${h.line}  ${h.item}  [${h.lint}]  ${h.text}`);
  console.error("");
}
if (stale.length) {
  console.error(`STALE ENTRY (${stale.length}) — approved site no longer exists; delete the entry:`);
  for (const a of stale) console.error(`  ${a.file}  ${a.item}  [${a.lint}]`);
  console.error("");
}
if (unreasoned.length) {
  console.error(`EMPTY REASON (${unreasoned.length}) — the reason IS the approval:`);
  for (const a of unreasoned) console.error(`  ${a.file}  ${a.item}  [${a.lint}]`);
  console.error("");
}
if (malformed.length) {
  console.error(`INCOMPLETE ENTRY (${malformed.length}) — needs file, item and lint:`);
  for (const a of malformed) console.error(`  ${JSON.stringify(a)}`);
  console.error("");
}
if (lowered.length) {
  console.error(`MANIFEST LOWERING (${lowered.length}) — suppresses the same diagnostic from further away:`);
  for (const l of lowered) console.error(`  ${l.file}${l.line ? `:${l.line}` : ""}  ${l.why}`);
  console.error("");
}
console.error("Fix the code — make it reachable, delete it, scope it to the build that uses it, or");
console.error("refactor the signature. `#[expect]` is not an alternative form. If a suppression is");
console.error(`genuinely warranted, it needs the repository owner's approval in ${ALLOWLIST}.`);
process.exit(1);
