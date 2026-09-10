import { test, expect } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { detectSequencer, detectSequencerState, resolveSkipState } from "./git-sequencer-state.mjs";
import { scrubGitEnv } from "./lib/run-git.mjs";

// The MARKER-VALIDITY tests below (state detection, forgery resistance) stay on the pure
// injectable-`exists`/`readFile`/`resolves` style: no real rebase/cherry-pick/merge/revert is
// driven, so every filesystem shape — including a forged one — is asserted without mutating a real
// repository's git dir. The direct-entry block's own `git rev-parse --git-dir` resolution is
// exercised only by actually running a hook under a real sequencer state, which this suite does
// not attempt.
//
// The REMAINING-WORK tests further down drive real git commands against real scratch
// repositories instead: a hand-written todo/counter file is exactly the shape this module is
// supposed to distrust, so "remaining work" is asserted against states git itself produced.

const SHA = "a".repeat(40);
const OTHER_SHA = "b".repeat(40);
const alwaysResolves = () => true;
const neverResolves = () => false;
const noConflicts = () => false;
const hasConflicts = () => true;

function fsFrom(files) {
  const norm = (p) => p.replace(/\\/g, "/");
  return {
    exists: (p) => norm(p) in files,
    readFile: (p) => {
      const v = files[norm(p)];
      if (v === undefined) throw new Error(`ENOENT: ${p}`);
      return v;
    },
  };
}

// Real-git fixture helpers for the remaining-work tests. Every direct git call below passes an
// explicit `env: scrubGitEnv()` — this test file runs from `gate:commit`'s pre-commit hook, so the
// test process itself can carry an enclosing commit's `GIT_INDEX_FILE`, and an unscrubbed fixture
// call would contend for that commit's in-flight index instead of touching its own scratch repo
// (see `scripts/lib/run-git.test.mjs`'s own header comment for the same rule applied there).

/**
 * A fresh scratch repository, `git`'s user identity configured. `defaultBranch` is read from
 * `git`'s own unborn `HEAD` right after `init` rather than assumed ("master" vs "main" differs by
 * the invoking machine's `init.defaultBranch`), so every fixture below names branches explicitly
 * instead of guessing which name the ambient git config picked.
 */
function initRepo() {
  const dir = mkdtempSync(join(tmpdir(), "seq-state-"));
  const env = scrubGitEnv();
  const run = (args) => execFileSync("git", args, { cwd: dir, env, encoding: "utf8" });
  run(["init", "-q"]);
  run(["config", "user.email", "test@example.com"]);
  run(["config", "user.name", "Test"]);
  const defaultBranch = run(["symbolic-ref", "--short", "HEAD"]).trim();
  return { dir, run, defaultBranch };
}

function writeAndCommit(run, dir, name, content, message) {
  writeFileSync(join(dir, name), content);
  run(["add", "-A"]);
  run(["commit", "-q", "-m", message]);
}

/** `git`'s own `.git` dir for `dir`, resolved the same way `resolveSkipState` does in production. */
function gitDirOf(run) {
  return run(["rev-parse", "--absolute-git-dir"]).trim();
}

test("reports no sequencer state when no marker is present", () => {
  const { exists, readFile } = fsFrom({});
  expect(detectSequencerState("/repo/.git", { exists, readFile, resolves: neverResolves })).toBe(
    null,
  );
});

test("detects an in-progress merge via a MERGE_HEAD holding a resolvable object id", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": `${SHA}\n` });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("merge");
});

test("detects a two-parent (octopus) merge: MERGE_HEAD holding two newline-separated resolvable ids", () => {
  const twoParents = `${SHA}\n${OTHER_SHA}\n`;
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": twoParents });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("merge");
});

test("detects a three-parent (octopus) merge: MERGE_HEAD holding three newline-separated resolvable ids", () => {
  const thirdParent = "c".repeat(40);
  const threeParents = `${SHA}\n${OTHER_SHA}\n${thirdParent}\n`;
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": threeParents });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("merge");
});

test("an octopus MERGE_HEAD is rejected if even ONE of its parent lines does not resolve — the forgery bar is per line, not relaxed", () => {
  const oneBadParent = `${SHA}\nnot-a-sha-at-all\n`;
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": oneBadParent });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe(null);
});

test("detects an in-progress cherry-pick via a CHERRY_PICK_HEAD holding a resolvable object id", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/CHERRY_PICK_HEAD": SHA });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("cherry-pick");
});

test("detects an in-progress revert via a REVERT_HEAD holding a resolvable object id", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/REVERT_HEAD": SHA });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("revert");
});

test("detects an interactive/merge rebase via rebase-merge/ carrying head-name and a resolving onto", () => {
  const { exists, readFile } = fsFrom({
    "/repo/.git/rebase-merge/head-name": "refs/heads/work\n",
    "/repo/.git/rebase-merge/onto": `${SHA}\n`,
  });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("rebase");
});

test("detects an am-backend rebase via rebase-apply/ carrying rebasing and a resolving onto", () => {
  const { exists, readFile } = fsFrom({
    "/repo/.git/rebase-apply/rebasing": "",
    "/repo/.git/rebase-apply/onto": SHA,
  });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe("rebase");
});

test("resolves markers under the given git dir, not a hardcoded .git", () => {
  // A worktree's git dir is a file pointing elsewhere (e.g. `.git/worktrees/<name>`); the
  // function must join against whatever path it is given, never assume the caller's cwd.
  const { exists, readFile } = fsFrom({ "/repo/.git/worktrees/feature/MERGE_HEAD": SHA });
  const state = detectSequencerState("/repo/.git/worktrees/feature", {
    exists,
    readFile,
    resolves: alwaysResolves,
  });
  expect(state).toBe("merge");
});

// Forgery-resistance cases — the property the coordinator's owner-ruled hardening exists for.
// None of these is a real git-written state, and none may be detected as one.

test("an EMPTY MERGE_HEAD (no content at all) is not detected", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": "" });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe(null);
});

test("a MERGE_HEAD containing arbitrary non-git text is not detected, even if resolves is never asked", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": "not-a-sha-at-all" });
  const resolves = () => {
    throw new Error("resolves() must not be called for shape-invalid content");
  };
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves });
  expect(state).toBe(null);
});

test("a MERGE_HEAD with the right SHAPE but an object that does not resolve is not detected", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": SHA });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: neverResolves });
  expect(state).toBe(null);
});

test("an EMPTY rebase-merge/ directory (no sibling files) is not detected", () => {
  // Mirrors the coordinator's reported forgery: `mkdir .git/rebase-merge` with nothing in it.
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/rebase-merge";
  const state = detectSequencerState("/repo/.git", {
    exists,
    readFile: () => {
      throw new Error("ENOENT");
    },
    resolves: alwaysResolves,
  });
  expect(state).toBe(null);
});

test("a rebase-merge/ carrying head-name but no onto file is not detected", () => {
  const { exists, readFile } = fsFrom({ "/repo/.git/rebase-merge/head-name": "refs/heads/work" });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe(null);
});

test("a rebase-apply/ without the rebasing marker (i.e. an in-progress `git am`, not a rebase) is not detected", () => {
  const { exists, readFile } = fsFrom({
    "/repo/.git/rebase-apply/applying": "",
    "/repo/.git/rebase-apply/onto": SHA,
  });
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves: alwaysResolves });
  expect(state).toBe(null);
});

test("resolveSkipState reports the detected state when the git-dir lookup succeeds AND the state has remaining work", () => {
  const execFile = () => "/repo/.git\n";
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": SHA });
  const result = resolveSkipState({
    execFile,
    exists,
    readFile,
    resolves: alwaysResolves,
    hasConflicts,
  });
  expect(result).toEqual({ determined: true, state: "merge" });
});

test("resolveSkipState reports null when a marker is present but has NO remaining work — the narrowed exemption", () => {
  const execFile = () => "/repo/.git\n";
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": SHA });
  const result = resolveSkipState({
    execFile,
    exists,
    readFile,
    resolves: alwaysResolves,
    hasConflicts: noConflicts,
  });
  expect(result).toEqual({ determined: true, state: null });
});

test("resolveSkipState reports the ordinary no-sequencer case as determined", () => {
  const execFile = () => "/repo/.git\n";
  const result = resolveSkipState({
    execFile,
    exists: () => false,
    resolves: neverResolves,
    hasConflicts: noConflicts,
  });
  expect(result).toEqual({ determined: true, state: null });
});

test("resolveSkipState fails toward RUNNING the tier, never toward skipping, when the git-dir lookup throws", () => {
  const execFile = () => {
    throw new Error("fatal: not a git repository (or any of the parent directories): .git");
  };
  const result = resolveSkipState({ execFile });
  expect(result.determined).toBe(false);
  expect(result.reason).toMatch(/not a git repository/);
  // The undetermined shape must carry no `state` a caller could mistake for "skip" — asserting
  // its shape directly is what would catch a future change that adds `state: null` back in and
  // lets a careless `if (!result.state)` read it as "no sequencer state, and also not skip", the
  // same ambiguity `determined` exists to remove.
  expect(result).not.toHaveProperty("state");
});

test("distinct object ids are handled independently — a resolves() keyed on the wrong id does not false-positive", () => {
  // Sanity check on the test helpers themselves: `resolves` must be asked about the CONTENT
  // actually read from the marker file, not a constant. A `resolves` that only accepts OTHER_SHA
  // must reject a marker holding SHA.
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": SHA });
  const resolves = (id) => id === OTHER_SHA;
  const state = detectSequencerState("/repo/.git", { exists, readFile, resolves });
  expect(state).toBe(null);
});

// Remaining-work tests, driven against real git-produced states — the owner-ruled narrowing this
// module exists for: a sequencer marker skips the commit tier only while GENUINE work remains
// (a queued todo step, or an unresolved conflict), never merely because the marker is present.
// Each state is exercised in BOTH directions: with remaining work (skips) and with nothing left
// (gates normally), each built with real git commands rather than a hand-written marker file.

test("rebase-merge: a queued step beyond the current conflict is remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "line1\n", "base");
  writeAndCommit(run, dir, "file.txt", "line1\nmainB\n", "mainB");
  run(["checkout", "-q", "-b", "work", "HEAD~1"]);
  writeAndCommit(run, dir, "file.txt", "line1\nw1\n", "w1");
  writeAndCommit(run, dir, "file.txt", "line1\nw1\nw2\n", "w2");
  // Rebase onto the branch carrying `mainB` — the first replayed commit conflicts, leaving the
  // second (`w2`) still queued in `git-rebase-todo`.
  try {
    execFileSync("git", ["rebase", defaultBranch], { cwd: dir, env: scrubGitEnv(), encoding: "utf8" });
  } catch {
    // A conflicting rebase always exits non-zero; the state on disk is what this test asserts.
  }
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "rebase", remainingWork: true });
});

test("rebase-merge: the LAST step, conflict resolved and staged, is NOT remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "line1\n", "base");
  writeAndCommit(run, dir, "file.txt", "line1\nmainB\n", "mainB");
  run(["checkout", "-q", "-b", "work", "HEAD~1"]);
  writeAndCommit(run, dir, "file.txt", "line1\nw1\n", "w1");
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["rebase", defaultBranch], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: the single replayed commit conflicts.
  }
  // Resolve and STAGE the conflict, but never run `--continue` — this is the exact instant an
  // ordinary `git rebase --continue`-driven commit is about to conclude the rebase.
  writeFileSync(join(dir, "file.txt"), "line1\nmainB\nw1\n");
  run(["add", "file.txt"]);
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "rebase", remainingWork: false });
});

test("rebase-apply: a queued patch beyond the current conflict (next < last) is remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "line1\n", "base");
  writeAndCommit(run, dir, "file.txt", "line1\nmainB\n", "mainB");
  run(["checkout", "-q", "-b", "work", "HEAD~1"]);
  writeAndCommit(run, dir, "file.txt", "line1\nw1\n", "w1");
  writeAndCommit(run, dir, "file.txt", "line1\nw2\n", "w2");
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["rebase", "--apply", defaultBranch], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: the first patch conflicts, leaving the second queued.
  }
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "rebase", remainingWork: true });
});

test("rebase-apply: the FINAL patch, conflict resolved and staged (next === last), is NOT remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "line1\n", "base");
  writeAndCommit(run, dir, "file.txt", "line1\nmainB\n", "mainB");
  run(["checkout", "-q", "-b", "work", "HEAD~1"]);
  writeAndCommit(run, dir, "file.txt", "line1\nw1\n", "w1");
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["rebase", "--apply", defaultBranch], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: the single patch conflicts.
  }
  writeFileSync(join(dir, "file.txt"), "line1\nmainB\nw1\n");
  run(["add", "file.txt"]);
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "rebase", remainingWork: false });
});

test("cherry-pick: a queued pick beyond the current conflict is remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "a\n", "base");
  run(["checkout", "-q", "-b", "feature"]);
  writeAndCommit(run, dir, "file.txt", "a\nc1\n", "c1");
  writeAndCommit(run, dir, "file.txt", "a\nc1\nc2\n", "c2");
  const c1 = run(["rev-parse", "feature~1"]).trim();
  const c2 = run(["rev-parse", "feature"]).trim();
  run(["checkout", "-q", defaultBranch]);
  writeAndCommit(run, dir, "file.txt", "a\nMAINCHANGE\n", "mainchange");
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["cherry-pick", c1, c2], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: the first pick conflicts, leaving the second queued.
  }
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "cherry-pick", remainingWork: true });
});

test("cherry-pick: a resolved-and-staged conflict with NOTHING queued beyond it is NOT remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "a\n", "base");
  run(["checkout", "-q", "-b", "feature"]);
  writeAndCommit(run, dir, "file.txt", "a\nc1\n", "c1");
  const c1 = run(["rev-parse", "feature"]).trim();
  run(["checkout", "-q", defaultBranch]);
  writeAndCommit(run, dir, "file.txt", "a\nMAINCHANGE\n", "mainchange");
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["cherry-pick", c1], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: the single cherry-pick conflicts.
  }
  writeFileSync(join(dir, "file.txt"), "a\nMAINCHANGE\nc1\n");
  run(["add", "file.txt"]);
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "cherry-pick", remainingWork: false });
});

test("revert: a queued revert beyond the current conflict is remaining work", () => {
  const { dir, run } = initRepo();
  writeAndCommit(run, dir, "file.txt", "a\n", "base");
  writeAndCommit(run, dir, "file.txt", "a\nb\n", "addb");
  writeAndCommit(run, dir, "file.txt", "a\nb\nc\n", "addc");
  writeAndCommit(run, dir, "file.txt", "a\nb\nc\nEDIT\n", "editafter");
  const addb = run(["rev-parse", "HEAD~2"]).trim();
  const addc = run(["rev-parse", "HEAD~1"]).trim();
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["revert", "--no-edit", addc, addb], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: reverting `addc` conflicts, leaving `addb`'s revert queued.
  }
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "revert", remainingWork: true });
});

test("revert: a single resolved-and-staged conflict with nothing queued beyond it is NOT remaining work", () => {
  const { dir, run } = initRepo();
  writeAndCommit(run, dir, "file.txt", "a\n", "base");
  writeAndCommit(run, dir, "file.txt", "a\nb\n", "addb");
  writeAndCommit(run, dir, "file.txt", "a\nb\nEDIT\n", "editafter");
  const addb = run(["rev-parse", "HEAD~1"]).trim();
  const env = scrubGitEnv();
  try {
    execFileSync("git", ["revert", "--no-edit", addb], { cwd: dir, env, encoding: "utf8" });
  } catch {
    // Expected: the single revert conflicts.
  }
  writeFileSync(join(dir, "file.txt"), "a\nEDIT\n");
  run(["add", "file.txt"]);
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "revert", remainingWork: false });
});

test("merge: an unresolved conflict is remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "a\n", "base");
  writeAndCommit(run, dir, "file.txt", "a\nmainb\n", "mainb");
  run(["checkout", "-q", "-b", "side", "HEAD~1"]);
  writeAndCommit(run, dir, "file.txt", "a\nsideb\n", "sideb");
  const env = scrubGitEnv();
  // Merge the branch carrying `mainb` into this conflicting `side` branch.
  try {
    execFileSync("git", ["merge", "--no-commit", "--no-ff", defaultBranch], {
      cwd: dir,
      env,
      encoding: "utf8",
    });
  } catch {
    // Expected: content conflict.
  }
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "merge", remainingWork: true });
});

test("merge: a clean `--no-commit --no-ff` merge with NOTHING to resolve is NOT remaining work", () => {
  const { dir, run, defaultBranch } = initRepo();
  writeAndCommit(run, dir, "file.txt", "a\n", "base");
  writeAndCommit(run, dir, "other.txt", "b\n", "otherfile");
  run(["checkout", "-q", "-b", "side", "HEAD~1"]);
  writeAndCommit(run, dir, "unrelated.txt", "c\n", "unrelated");
  const env = scrubGitEnv();
  execFileSync("git", ["merge", "--no-commit", "--no-ff", defaultBranch], {
    cwd: dir,
    env,
    encoding: "utf8",
  });
  const gitDir = gitDirOf(run);
  const detected = detectSequencer(gitDir, {});
  expect(detected).toEqual({ state: "merge", remainingWork: false });
});
