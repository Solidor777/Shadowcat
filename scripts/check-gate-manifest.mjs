// Fails when scripts/gates.toml and .github/workflows/ci.yml disagree about which gates exist.
//
// The gate list is otherwise prose: twenty-odd `run:` steps an agent must transcribe by hand,
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

const JOB = /^ {2}([A-Za-z0-9_-]+):\s*$/;
const RUN_INLINE = /^\s*-?\s*run:\s*(\S.*)$/;
const RUN_BLOCK = /^(\s*)-?\s*run:\s*[|>][-+]?\s*$/;

/** Every `run:` step in the workflow, with the job that contains it. */
export function parseWorkflowRunSteps(yamlText) {
  const lines = yamlText.split("\n").map((l) => l.replace(/\r$/, ""));
  const out = [];
  let job = "";
  for (let i = 0; i < lines.length; i++) {
    const j = lines[i].match(JOB);
    if (j) {
      job = j[1];
      continue;
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
      out.push({ job, command: normCommand(body.join(" ")), line: i + 1 });
      i = k - 1;
      continue;
    }
    const inline = lines[i].match(RUN_INLINE);
    if (inline) out.push({ job, command: normCommand(inline[1]), line: i + 1 });
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

const key = (x) => `${x.job} ${x.command}`;

/** Workflow steps with no entry, entries with no step, and ci-only entries with no reason. */
export function diffManifest(steps, entries) {
  const byEntry = new Set(entries.map(key));
  const byStep = new Set(steps.map(key));
  return {
    unclassified: steps.filter((s) => !byEntry.has(key(s))),
    stale: entries.filter((e) => !byStep.has(key(e))),
    missingReason: entries.filter((e) => e.tier === "ci-only" && e.reason.trim() === ""),
  };
}

if (isDirectEntry(import.meta.url)) {
  const steps = parseWorkflowRunSteps(readFileSync(WORKFLOW, "utf8"));
  const entries = parseGateManifest(readFileSync(MANIFEST, "utf8"), MANIFEST);
  const d = diffManifest(steps, entries);
  const bad = d.unclassified.length + d.stale.length + d.missingReason.length;
  for (const s of d.unclassified) {
    console.error(`${WORKFLOW}:${s.line}: [${s.job}] unclassified gate: ${s.command}`);
  }
  for (const e of d.stale) {
    console.error(`${MANIFEST}:${e.line}: [${e.job}] entry names no workflow step: ${e.command}`);
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
