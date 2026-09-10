// Fails when scripts/gates.toml and the workflows under .github/workflows/ disagree about which
// gates exist.
//
// The gate list is otherwise prose: every `run:` step in every workflow, transcribed by hand,
// and a transcription that can drop an entry eventually drops one. Classifying every step here
// makes the list an artifact, and makes divergence from CI a gate in its own right — a new
// workflow step that nobody classified fails at the next commit rather than at the next push.
//
// INVARIANT: every `run:` step in every workflow file has exactly one manifest entry, keyed by
// (workflow file, job, normalised command). A command repeated across jobs or across files is a
// distinct entry per (file, job). The file set is ENUMERATED from the directory, never listed
// here: a second workflow file arrives with every one of its steps unclassified, and this check
// says so, rather than sitting outside a hardcoded path with nothing to report it.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export const MANIFEST = "scripts/gates.toml";
export const WORKFLOWS_DIR = ".github/workflows";
export const TIERS = ["commit", "push", "setup", "ci-only"];
// A tier that does not run locally is a CLAIM about the step — "cannot run here" or "is not a
// gate" — and an unreasoned claim is the one made under pressure: re-tiering a checker to
// `setup` silently stops it running locally while the drift check still reports zero
// discrepancies. So both non-running tiers carry a `reason`, checked the same way.
export const REASONED_TIERS = ["setup", "ci-only"];

/** Every workflow file GitHub Actions would read from `dir`: `*.yml` and `*.yaml`, sorted. */
export function listWorkflowFiles(dir) {
  return readdirSync(dir)
    .filter((name) => /\.ya?ml$/i.test(name))
    .sort();
}

/** Collapses whitespace runs so a `run: |` block and its manifest entry compare equal. */
export function normCommand(text) {
  return text.replace(/\s+/g, " ").trim();
}

// A bare 2-space-indented `key:` is ambiguous on its own — `on:`'s `push:`/`pull_request:` have
// the identical shape to a job name. TOP_KEY tracks which 0-indent block we are in, and JOB is
// only consulted while that block is `jobs:`, so a same-shaped key elsewhere in the document is
// never mistaken for a job. TOP_KEY matches a key with or without an inline value, since a
// workflow-level `env: { ... }` must both leave `jobs:` and register as environment.
const TOP_KEY = /^([A-Za-z0-9_-]+):(?:\s|$)/;
const JOB = /^ {2}([A-Za-z0-9_-]+):\s*$/;
// Job-level keys sit two columns deeper than the job name JOB matches.
const JOB_ENV = /^ {4}env:(?:\s|$)/;
const STEP_START = /^(\s*)-\s+/;
const ENV_KEY = /^(\s*)(?:-\s+)?env:(?:\s|$)/;
const RUN_INLINE = /^\s*-?\s*run:\s*(\S.*)$/;
const RUN_BLOCK = /^(\s*)-?\s*run:\s*[|>][-+]?\s*$/;

/**
 * Every `run:` step in one workflow file, with the job that contains it and the two facts about
 * its SHAPE that `normCommand` erases: whether the source was a multi-line block (`multiline`),
 * and whether an `env:` block applies to it (`env`) — the step's own, its job's, or the
 * workflow's. Both are read here, at parse time, because neither survives into the command
 * string a manifest entry is keyed on.
 *
 * An `env:` key counts only at the column where a step's own keys sit (two past the list dash),
 * so the same key nested under a step's `with:` is an action input, not the step's environment.
 * A step's `env:` written AFTER its `run:` is attributed retroactively to the step already
 * emitted.
 */
export function parseWorkflowRunSteps(yamlText, workflow) {
  const lines = yamlText.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let job = "";
  let inJobs = false;
  let workflowEnv = false;
  let jobEnv = false;
  // The list item whose keys are currently being read: the column of its dash, whether it has
  // declared an `env:` of its own, and the step object emitted for its `run:` (if seen yet).
  let step = null;
  for (let i = 0; i < lines.length; i++) {
    const top = lines[i].match(TOP_KEY);
    if (top) {
      inJobs = top[1] === "jobs";
      if (top[1] === "env") workflowEnv = true;
      step = null;
      continue;
    }
    if (inJobs) {
      const j = lines[i].match(JOB);
      if (j) {
        job = j[1];
        jobEnv = false;
        step = null;
        continue;
      }
      if (JOB_ENV.test(lines[i])) {
        jobEnv = true;
        continue;
      }
    }
    const start = lines[i].match(STEP_START);
    if (start && (step === null || start[1].length <= step.dash)) {
      step = { dash: start[1].length, env: false, run: null };
    }
    const envKey = lines[i].match(ENV_KEY);
    if (envKey && step !== null) {
      const col = envKey[1].length + (start ? start[0].length - start[1].length : 0);
      if (col === step.dash + 2) {
        step.env = true;
        if (step.run) step.run.env = true;
      }
      continue;
    }
    const env = workflowEnv || jobEnv || (step !== null && step.env);
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
      const emitted = { workflow, job, command: normCommand(body.join(" ")), line: i + 1, multiline: true, env };
      out.push(emitted);
      if (step !== null) step.run = emitted;
      i = k - 1;
      continue;
    }
    const inline = lines[i].match(RUN_INLINE);
    if (inline) {
      const emitted = { workflow, job, command: normCommand(inline[1]), line: i + 1, multiline: false, env };
      out.push(emitted);
      if (step !== null) step.run = emitted;
    }
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
      cur = { workflow: "", job: "", command: "", tier: "", reason: "", line: i + 1 };
      out.push(cur);
      continue;
    }
    // `(?:[^"\\]|\\.)*` allows an escaped quote inside the value (several real commands quote
    // a shell argument), so the match's closing `"` is the value's own, not the first embedded one.
    const kv = line.match(/^(workflow|job|command|tier|reason)\s*=\s*"((?:[^"\\]|\\.)*)"$/);
    if (!kv || !cur) {
      throw new Error(
        `${sourceName}:${i + 1}: cannot parse. Expected [[gate]] or workflow/job/command/tier/reason = "value". got: ${raw}`,
      );
    }
    const value = kv[2].replace(/\\"/g, '"');
    cur[kv[1]] = kv[1] === "command" ? normCommand(value) : value;
  }
  for (const e of out) {
    if (e.workflow === "") {
      throw new Error(`${sourceName}:${e.line}: entry needs a workflow (the file under ${WORKFLOWS_DIR} its step lives in)`);
    }
    if (!TIERS.includes(e.tier)) {
      throw new Error(
        `${sourceName}:${e.line}: tier must be one of ${TIERS.join(", ")}. got: ${e.tier || "(none)"}`,
      );
    }
  }
  return out;
}

// A null byte cannot occur in a file name, a YAML job name or a shell command, so the key is
// collision-proof by construction; a plain space is not (a job or command could itself contain one).
const key = (x) => `${x.workflow}\u0000${x.job}\u0000${x.command}`;

// Four shapes are not executable locally by construction, and tiering any of them `commit` or
// `push` is always a defect. The fix in every case is the same: extract the step into a script
// both CI and the local runner invoke (`check-binary-size.mjs` and `cargo-doc-strict.mjs` are the
// precedents), then tier the one-line invocation.
//   - an unresolved GitHub Actions expression (`${{ ... }}` cannot occur outside an
//     Actions-only string, so this has zero false positives on a real one-liner) — the
//     binary-size step was exactly this: a `run: |` block naming the binary through
//     `${{ runner.os == 'Windows' && '.exe' || '' }}`, folded by `normCommand` into one line of
//     concatenated fragments rather than the three statements it actually runs
//   - a step whose source was a multi-line `run: |`/`run: >` block — `parseWorkflowRunSteps`
//     tags this at parse time (`multiline: true`), before `normCommand` erases the distinction
//   - a step under an `env:` block (its own, its job's, or the workflow's) — the manifest stores
//     the command string alone and the local runner applies no per-step environment, so the same
//     string runs locally with the variable unset. A lint the step denies through `RUSTDOCFLAGS`
//     passes locally and fails in CI, which is the exact defect this whole check exists to remove.
//     `parseWorkflowRunSteps` tags this too (`env: true`), since nothing in the command string
//     carries it.
export const UNRESOLVED_EXPRESSION = /\$\{\{/;

// A fourth shape needs no `${{ }}` at all: a plain single-line `run:` step that reads an
// environment variable the Actions RUNNER injects and a local shell never sets — `$GITHUB_ENV`,
// `$GITHUB_OUTPUT`, `$GITHUB_STEP_SUMMARY`, `$GITHUB_WORKSPACE`, `$RUNNER_OS`, `$RUNNER_TEMP` are
// the named instances found in this workflow's history, all injected by the same runner runtime.
// Rather than enumerate (which only ever catches names already seen), this matches the whole
// `GITHUB_*`/`RUNNER_*` family — every member arrives the same way, so the same defect recurs
// under any of their names (`GITHUB_SHA`, `RUNNER_ARCH`, ...) without a matching entry here.
// `${VAR}`/`${VAR:-default}` and bare `$VAR` both match; the leading `\$\{?` is optional so
// either form is caught.
export const RUNNER_ONLY_VAR = /\$\{?(?:GITHUB_|RUNNER_)[A-Z_]+/;

/** Commands tiered `commit`/`push` that cannot execute locally: expression, runner-only env var, multi-line block, or `env:` block. */
export function unrunnableLocalEntries(steps, entries) {
  const blockKeys = new Set(steps.filter((s) => s.multiline).map(key));
  const envKeys = new Set(steps.filter((s) => s.env).map(key));
  return entries.filter(
    (e) =>
      (e.tier === "commit" || e.tier === "push") &&
      (UNRESOLVED_EXPRESSION.test(e.command) ||
        RUNNER_ONLY_VAR.test(e.command) ||
        blockKeys.has(key(e)) ||
        envKeys.has(key(e))),
  );
}

/** Workflow steps with no entry, entries with no step, reasoned-tier entries with no reason, and local-tier entries that cannot run locally. */
export function diffManifest(steps, entries) {
  const byEntry = new Set(entries.map(key));
  const byStep = new Set(steps.map(key));
  return {
    unclassified: steps.filter((s) => !byEntry.has(key(s))),
    stale: entries.filter((e) => !byStep.has(key(e))),
    missingReason: entries.filter((e) => REASONED_TIERS.includes(e.tier) && e.reason.trim() === ""),
    unrunnableLocal: unrunnableLocalEntries(steps, entries),
  };
}

if (isDirectEntry(import.meta.url)) {
  const files = listWorkflowFiles(WORKFLOWS_DIR);
  const steps = files.flatMap((f) => parseWorkflowRunSteps(readFileSync(join(WORKFLOWS_DIR, f), "utf8"), f));
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  const d = diffManifest(steps, entries);
  const bad =
    d.unclassified.length + d.stale.length + d.missingReason.length + d.unrunnableLocal.length;
  for (const s of d.unclassified) {
    console.error(`${WORKFLOWS_DIR}/${s.workflow}:${s.line}: [${s.job}] unclassified gate: ${s.command}`);
  }
  for (const e of d.stale) {
    console.error(`${MANIFEST}:${e.line}: [${e.workflow} ${e.job}] entry names no workflow step: ${e.command}`);
  }
  for (const e of d.unrunnableLocal) {
    console.error(
      `${MANIFEST}:${e.line}: [${e.workflow} ${e.job}] tier "${e.tier}" entry cannot run locally (unresolved expression, runner-only env var, multi-line block, or an env: block the local runner never applies): ${e.command}. Extract it into a script both CI and the local runner invoke.`,
    );
  }
  for (const e of d.missingReason) {
    console.error(`${MANIFEST}:${e.line}: ${e.tier} entry needs a reason: ${e.command}`);
  }
  if (bad > 0) {
    console.error(
      `\nlint:gate-manifest: ${bad} discrepancy(ies). Classify each workflow step in ${MANIFEST}.`,
    );
    process.exit(1);
  }
  console.log(
    `lint:gate-manifest: ${files.length} workflow file(s), ${steps.length} workflow step(s), ${entries.length} entry(ies), 0 error(s)`,
  );
}
