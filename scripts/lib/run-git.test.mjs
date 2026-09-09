import { test, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { runGit } from "./run-git.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

test("runGit returns the trimmed stdout on success", () => {
  const execFile = () => "  abc123  \n";
  const result = runGit(["rev-parse", "HEAD"], "HEAD", { execFile });
  expect(result).toEqual({ ok: true, stdout: "abc123" });
});

test("runGit reports a legible failure naming the command and what could not be determined, never a raw exception", () => {
  const execFile = () => {
    throw new Error("fatal: not a git repository");
  };
  const result = runGit(["rev-parse", "--git-dir"], "the sequencer state", { execFile });
  expect(result.ok).toBe(false);
  expect(result.message).toContain("the sequencer state");
  expect(result.message).toContain("git rev-parse --git-dir");
  expect(result.message).toContain("fatal: not a git repository");
});

// Source-scanning regression test, same precedent as run-gate-tier.test.mjs's GIT-CALL-BUDGET
// check: the git-hook installer, the sequencer probe, and the tier runner each fixed one
// unguarded `execFileSync("git", ...)` at their own entry point separately, one file at a time —
// the site was never the cause, so a check that only re-verifies the site already fixed proves
// nothing about a fourth recurring somewhere else across these files. This scans every file the
// git-hook mechanism installs or ships.
//
// Deliberately scoped to the git-hook installer, the sequencer probe, and the tier runner, not
// the whole repo: `check-aria-label-keys.mjs`, `check-file-lines.mjs` and
// `check-inline-tests.mjs` also call `execFileSync("git", ...)` unguarded, but they predate the
// git-hooks/receipt mechanism and are explicitly out of scope for it — left for the owner to
// address separately, not folded into a wider check here.
const GIT_HOOK_ENTRY_POINTS = [
  "scripts/install-git-hooks.mjs",
  "scripts/git-sequencer-state.mjs",
  "scripts/run-gate-tier.mjs",
];

test("no git-hook entry point calls execFileSync(\"git\", ...) directly outside the shared runner", () => {
  const offenders = [];
  for (const rel of GIT_HOOK_ENTRY_POINTS) {
    const text = readFileSync(resolve(REPO_ROOT, rel), "utf8");
    if (/execFileSync\(\s*["']git["']/.test(text)) offenders.push(rel);
  }
  expect(offenders).toEqual([]);
});
