import { test, expect } from "vitest";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, relative, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runGit } from "./run-git.mjs";
import { sources, norm } from "./gate-corpus.mjs";

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
// unguarded `execFileSync` git call at their own entry point separately, one file at a time — the
// site was never the cause, so a check that only re-verifies the site already fixed proves
// nothing about a further one recurring somewhere else. This scans every `.mjs` file under
// `scripts/`, not just the three files fixed so far: an INCLUDE-list naming those three would
// make a fourth future entry point invisible to the check until someone remembers to add it,
// which fails toward missing a new violation. An EXCLUDE-list fails the other way — a new file is
// scanned and must earn its way OUT — which is the direction a recurrence guard needs.
//
// Test files (`*.test.mjs`) are excluded by NAME, not enumerated: they legitimately spin up
// throwaway git repositories as fixtures (e.g. `check-skill-symbol-refs*.test.mjs`), and this
// file's own prose above necessarily spells out the literal pattern it scans for — an
// include-everything-except-tests list would have to name every current and future test file to
// stay accurate, while a name-based rule does not decay.
//
// The four production files below predate the git-hook installer, sequencer probe, and tier
// runner and are explicitly out of scope for this check, left for the owner to address
// separately: `check-aria-label-keys.mjs`, `check-file-lines.mjs`, and `check-inline-tests.mjs`
// are pre-existing doc/lint checkers unrelated to git hooks; `lib/gate-corpus.mjs` is the
// pre-existing skill/doc-gate corpus helper, which already wraps its one git call in its own
// try/catch (returning `null` on failure) independently of this mechanism.
const EXCLUDED_FILES = new Set([
  "scripts/check-aria-label-keys.mjs",
  "scripts/check-file-lines.mjs",
  "scripts/check-inline-tests.mjs",
  "scripts/lib/gate-corpus.mjs",
]);

/** True when `text` (a file's full contents) contains a direct, unguarded `execFileSync("git", ...)` call. */
function callsGitUnguarded(text) {
  return /execFileSync\(\s*["']git["']/.test(text);
}

/**
 * Every `.mjs` file under `scriptsRoot` (paths reported relative to `repoRoot`) that calls git
 * unguarded, excluding test files by name and `EXCLUDED_FILES` by documented exception.
 */
function findUnguardedGitCallers(scriptsRoot, repoRoot) {
  const offenders = [];
  for (const path of sources(scriptsRoot, [".mjs"])) {
    const rel = norm(relative(repoRoot, path));
    if (rel.endsWith(".test.mjs")) continue;
    if (EXCLUDED_FILES.has(rel)) continue;
    if (callsGitUnguarded(readFileSync(path, "utf8"))) offenders.push(rel);
  }
  return offenders;
}

test("callsGitUnguarded matches a direct execFileSync git call in either quote style, not an unrelated command", () => {
  expect(callsGitUnguarded('execFileSync("git", ["status"], {})')).toBe(true);
  expect(callsGitUnguarded("execFileSync('git', ['status'], {})")).toBe(true);
  expect(callsGitUnguarded('execFileSync("npm", ["install"], {})')).toBe(false);
});

test("no non-test .mjs file under scripts/ calls git unguarded outside the shared runner or the documented exceptions", () => {
  const offenders = findUnguardedGitCallers(resolve(REPO_ROOT, "scripts"), REPO_ROOT);
  expect(offenders).toEqual([]);
});

test("findUnguardedGitCallers catches a NEW file containing the pattern without it being named anywhere first", () => {
  // Proves the exclude-list design: a file that exists in neither `EXCLUDED_FILES` nor any other
  // list is scanned and flagged by default, which is the property an include-list design (naming
  // three known files) cannot have for a file nobody has named yet.
  const scratchRepoRoot = mkdtempSync(join(tmpdir(), "run-git-scan-"));
  const scriptsDir = join(scratchRepoRoot, "scripts");
  mkdirSync(scriptsDir, { recursive: true });
  writeFileSync(
    join(scriptsDir, "brand-new-entry-point.mjs"),
    'execFileSync("git", ["status"], { encoding: "utf8" });\n',
  );
  const offenders = findUnguardedGitCallers(scriptsDir, scratchRepoRoot);
  expect(offenders).toEqual(["scripts/brand-new-entry-point.mjs"]);
});
