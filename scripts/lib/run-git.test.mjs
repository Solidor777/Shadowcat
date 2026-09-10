import { test, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { runGit } from "./run-git.mjs";
import { norm } from "./gate-corpus.mjs";

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
// site was never the cause, so a check that only re-verifies the site already fixed proves nothing
// about a further one recurring somewhere else.
//
// Enumerates via `git ls-files` rather than walking a named directory tree: a directory-walk scan
// root is itself an include-list one level up from the file-level exclude-list below it — it saw
// every file under `scripts/`, but a git-invoking entry point living anywhere else in the tracked
// tree (a Claude Code hook under `.claude/hooks/`, or any future location) was invisible to it
// until someone remembered to widen the root. Tracked-file enumeration has no location to omit:
// every `.mjs`/`.js` file git tracks, anywhere in the repository, is in scope by construction, and
// ignored directories (`node_modules`, `dist`, build output) and untracked scratch content are
// already excluded because they were never tracked.
//
// Test files (`*.test.mjs`/`*.test.js`) are excluded by NAME, not enumerated: they legitimately
// spin up throwaway git repositories as fixtures (e.g. `check-skill-symbol-refs*.test.mjs`, and
// this file's own fixture below), and this file's own prose necessarily spells out the literal
// pattern it scans for — an include-everything-except-tests list would have to name every current
// and future test file to stay accurate, while a name-based rule does not decay.
//
// The four production files below predate the git-hook installer, sequencer probe, and tier
// runner and are explicitly out of scope for this check, left for the owner to address separately:
// `check-aria-label-keys.mjs`, `check-file-lines.mjs`, and `check-inline-tests.mjs` are
// pre-existing doc/lint checkers unrelated to git hooks; `lib/gate-corpus.mjs` is the pre-existing
// skill/doc-gate corpus helper, which already wraps its one git call in its own try/catch
// (returning `null` on failure) independently of this mechanism.
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

/** Every `.mjs`/`.js` path `git ls-files` reports under `repoRoot`, NUL-separated and unfiltered. */
function listTrackedFiles(repoRoot) {
  const execFile = (cmd, args, opts) =>
    execFileSync(cmd, args, { ...opts, cwd: repoRoot, maxBuffer: 64 * 1024 * 1024 });
  const result = runGit(["ls-files", "-z"], "the tracked file list", { execFile });
  if (!result.ok) throw new Error(result.message);
  return result.stdout.split("\0").filter(Boolean);
}

/**
 * Every git-tracked `.mjs`/`.js` file anywhere under `repoRoot` (paths reported relative to
 * `repoRoot`) that calls git unguarded, excluding test files by name and `EXCLUDED_FILES` by
 * documented exception.
 */
function findUnguardedGitCallers(repoRoot) {
  const offenders = [];
  for (const entry of listTrackedFiles(repoRoot)) {
    const rel = norm(entry);
    if (!(rel.endsWith(".mjs") || rel.endsWith(".js"))) continue;
    if (rel.endsWith(".test.mjs") || rel.endsWith(".test.js")) continue;
    if (EXCLUDED_FILES.has(rel)) continue;
    if (callsGitUnguarded(readFileSync(join(repoRoot, entry), "utf8"))) offenders.push(rel);
  }
  return offenders;
}

test("callsGitUnguarded matches a direct execFileSync git call in either quote style, not an unrelated command", () => {
  expect(callsGitUnguarded('execFileSync("git", ["status"], {})')).toBe(true);
  expect(callsGitUnguarded("execFileSync('git', ['status'], {})")).toBe(true);
  expect(callsGitUnguarded('execFileSync("npm", ["install"], {})')).toBe(false);
});

test("no tracked non-test .mjs/.js file calls git unguarded outside the shared runner or the documented exceptions", () => {
  const offenders = findUnguardedGitCallers(REPO_ROOT);
  expect(offenders).toEqual([]);
});

test("findUnguardedGitCallers catches a NEW file regardless of which directory it lives in, without it being named anywhere first", () => {
  // Proves the enumerate-the-corpus design: a file living outside `scripts/` entirely — the exact
  // shape of gap a directory-walk root cannot see — is scanned and flagged by default, which is
  // the property a location-list design (naming `scripts/`, or any other directory, as the root)
  // cannot have for a location nobody has named yet.
  const scratchRepoRoot = mkdtempSync(join(tmpdir(), "run-git-scan-"));
  execFileSync("git", ["init", "-q"], { cwd: scratchRepoRoot });
  execFileSync("git", ["config", "user.email", "test@example.com"], { cwd: scratchRepoRoot });
  execFileSync("git", ["config", "user.name", "Test"], { cwd: scratchRepoRoot });
  const hooksDir = join(scratchRepoRoot, "some", "other", "location");
  mkdirSync(hooksDir, { recursive: true });
  writeFileSync(
    join(hooksDir, "brand-new-entry-point.mjs"),
    'execFileSync("git", ["status"], { encoding: "utf8" });\n',
  );
  execFileSync("git", ["add", "-A"], { cwd: scratchRepoRoot });
  const offenders = findUnguardedGitCallers(scratchRepoRoot);
  expect(offenders).toEqual(["some/other/location/brand-new-entry-point.mjs"]);
});
