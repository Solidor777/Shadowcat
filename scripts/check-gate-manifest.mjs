// Fails when scripts/gates.toml and .github/workflows/ci.yml disagree about which gates exist.
//
// The gate list is otherwise prose: every `run:` step in the workflow, transcribed by hand,
// and a transcription that can drop an entry eventually drops one. Classifying every step here
// makes the list an artifact, and makes divergence from CI a gate in its own right — a new
// workflow step that nobody classified fails at the next commit rather than at the next push.
//
// INVARIANT: every `run:` step in the workflow has exactly one manifest entry, keyed by
// (job, normalised command). A command repeated across jobs is a distinct entry per job.

import { readFileSync } from "node:fs";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export const MANIFEST = "scripts/gates.toml";
export const WORKFLOW = ".github/workflows/ci.yml";
export const TIERS = ["commit", "push", "setup", "ci-only"];

/** Collapses whitespace runs so a `run: |` block and its manifest entry compare equal. */
export function normCommand(text) {
  return text.replace(/\s+/g, " ").trim();
}

// A bare 2-space-indented `key:` is ambiguous on its own — `on:`'s `push:`/`pull_request:` have
// the identical shape to a job name. TOP_LEVEL tracks which 0-indent block we are in, and JOB is
// only consulted while that block is `jobs:`, so a same-shaped key elsewhere in the document is
// never mistaken for a job.
const TOP_LEVEL = /^([A-Za-z0-9_-]+):\s*$/;
const JOB = /^ {2}([A-Za-z0-9_-]+):\s*$/;
const RUN_INLINE = /^\s*-?\s*run:\s*(\S.*)$/;
const RUN_BLOCK = /^(\s*)-?\s*run:\s*[|>][-+]?\s*$/;

/** Every `run:` step in the workflow, with the job that contains it. */
export function parseWorkflowRunSteps(yamlText) {
  const lines = yamlText.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let job = "";
  let inJobs = false;
  for (let i = 0; i < lines.length; i++) {
    const top = lines[i].match(TOP_LEVEL);
    if (top) {
      inJobs = top[1] === "jobs";
      continue;
    }
    if (inJobs) {
      const j = lines[i].match(JOB);
      if (j) {
        job = j[1];
        continue;
      }
    }
    const block = lines[i].match(RUN_BLOCK);
    if (block) {
      const indent = block[1].length;
      const body = [];
      let k = i + 1;
      for (; k < lines.length; k++) {
        const bare = lines[k].trim();
        if (bare === "") {
          body.push("");
          continue;
        }
        if (lines[k].search(/\S/) <= indent) break;
        body.push(bare);
      }
      out.push({ job, command: normCommand(body.join(" ")), line: i + 1, multiline: true });
      i = k - 1;
      continue;
    }
    const inline = lines[i].match(RUN_INLINE);
    if (inline) out.push({ job, command: normCommand(inline[1]), line: i + 1, multiline: false });
  }
  return out;
}

/**
 * Parses the restricted `[[gate]]` TOML subset the manifest uses.
 *
 * A comment is a line whose FIRST non-blank character is `#` — matching `parseAllowlist`'s
 * house style — rather than anything after a bare `#` on the line. Several real CI commands
 * (the binary-size-budget block) carry a `#` inside their own quoted value; a trailing-comment
 * strip would truncate the value at that character instead of at the line's actual end.
 */
export function parseGateManifest(text, sourceName) {
  const lines = text.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let cur = null;
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    if (line === "[[gate]]") {
      cur = { job: "", command: "", tier: "", reason: "", line: i + 1 };
      out.push(cur);
      continue;
    }
    // `(?:[^"\\]|\\.)*` allows an escaped quote inside the value (several real commands quote
    // a shell argument), so the match's closing `"` is the value's own, not the first embedded one.
    const kv = line.match(/^(job|command|tier|reason)\s*=\s*"((?:[^"\\]|\\.)*)"$/);
    if (!kv || !cur) {
      throw new Error(
        `${sourceName}:${i + 1}: cannot parse. Expected [[gate]] or job/command/tier/reason = "value". got: ${raw}`,
      );
    }
    const value = kv[2].replace(/\\"/g, '"');
    cur[kv[1]] = kv[1] === "command" ? normCommand(value) : value;
  }
  for (const e of out) {
    if (!TIERS.includes(e.tier)) {
      throw new Error(
        `${sourceName}:${e.line}: tier must be one of ${TIERS.join(", ")}. got: ${e.tier || "(none)"}`,
      );
    }
  }
  return out;
}

// A null byte cannot occur in a YAML job name or a shell command, so the key is collision-proof
// by construction; a plain space is not (a job or command could itself contain one).
const key = (x) => `${x.job}\u0000${x.command}`;

// Two shapes are not executable locally by construction, and tiering either `commit` or `push`
// is always a defect — the binary-size step was exactly this: a `run: |` block naming the
// binary through `${{ runner.os == 'Windows' && '.exe' || '' }}`, an expression that resolves
// only inside Actions, folded by `normCommand` into one line of concatenated fragments rather
// than the three statements it actually runs. The fix in both cases is the same: extract the
// step into a script both CI and the local runner invoke (`check-binary-size.mjs` is the
// precedent), then tier the one-line invocation.
//   - an unresolved GitHub Actions expression (`${{ ... }}` cannot occur outside an
//     Actions-only string, so this has zero false positives on a real one-liner)
//   - a step whose source was a multi-line `run: |`/`run: >` block — `parseWorkflowRunSteps`
//     tags this at parse time (`multiline: true`), before `normCommand` erases the distinction
export const UNRESOLVED_EXPRESSION = /\$\{\{/;

// A third shape needs no `${{ }}` at all: a plain single-line `run:` step that reads an
// environment variable the Actions RUNNER injects and a local shell never sets — `$GITHUB_ENV`,
// `$GITHUB_OUTPUT`, `$GITHUB_STEP_SUMMARY`, `$GITHUB_WORKSPACE`, `$RUNNER_OS`, `$RUNNER_TEMP` are
// the named instances found in this workflow's history, all injected by the same runner runtime.
// Rather than enumerate (which only ever catches names already seen), this matches the whole
// `GITHUB_*`/`RUNNER_*` family — every member arrives the same way, so the same defect recurs
// under any of their names (`GITHUB_SHA`, `RUNNER_ARCH`, ...) without a matching entry here.
// `${VAR}`/`${VAR:-default}` and bare `$VAR` both match; the leading `\$\{?` is optional so
// either form is caught.
export const RUNNER_ONLY_VAR = /\$\{?(?:GITHUB_|RUNNER_)[A-Z_]+/;

/** Commands tiered `commit`/`push` that cannot execute locally: expression, runner-only env var, or multi-line block. */
export function unrunnableLocalEntries(steps, entries) {
  const blockKeys = new Set(steps.filter((s) => s.multiline).map(key));
  return entries.filter(
    (e) =>
      (e.tier === "commit" || e.tier === "push") &&
      (UNRESOLVED_EXPRESSION.test(e.command) ||
        RUNNER_ONLY_VAR.test(e.command) ||
        blockKeys.has(key(e))),
  );
}

/** Workflow steps with no entry, entries with no step, and ci-only entries with no reason. */
export function diffManifest(steps, entries) {
  const byEntry = new Set(entries.map(key));
  const byStep = new Set(steps.map(key));
  return {
    unclassified: steps.filter((s) => !byEntry.has(key(s))),
    stale: entries.filter((e) => !byStep.has(key(e))),
    missingReason: entries.filter((e) => e.tier === "ci-only" && e.reason.trim() === ""),
    unrunnableLocal: unrunnableLocalEntries(steps, entries),
  };
}

if (isDirectEntry(import.meta.url)) {
  const steps = parseWorkflowRunSteps(readFileSync(WORKFLOW, "utf8"));
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  const d = diffManifest(steps, entries);
  const bad =
    d.unclassified.length + d.stale.length + d.missingReason.length + d.unrunnableLocal.length;
  for (const s of d.unclassified) {
    console.error(`${WORKFLOW}:${s.line}: [${s.job}] unclassified gate: ${s.command}`);
  }
  for (const e of d.stale) {
    console.error(`${MANIFEST}:${e.line}: [${e.job}] entry names no workflow step: ${e.command}`);
  }
  for (const e of d.unrunnableLocal) {
    console.error(
      `${MANIFEST}:${e.line}: [${e.job}] tier "${e.tier}" entry cannot run locally (unresolved expression, runner-only env var, or multi-line block): ${e.command}. Extract it into a script both CI and the local runner invoke.`,
    );
  }
  for (const e of d.missingReason) {
    console.error(`${MANIFEST}:${e.line}: ci-only entry needs a reason: ${e.command}`);
  }
  if (bad > 0) {
    console.error(
      `\nlint:gate-manifest: ${bad} discrepancy(ies). Classify each workflow step in ${MANIFEST}.`,
    );
    process.exit(1);
  }
  console.log(`lint:gate-manifest: ${steps.length} workflow step(s), ${entries.length} entry(ies), 0 error(s)`);
}
