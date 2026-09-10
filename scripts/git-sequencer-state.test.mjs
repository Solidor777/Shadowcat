import { test, expect } from "vitest";
import { detectSequencerState, resolveSkipState } from "./git-sequencer-state.mjs";

// A real rebase/cherry-pick/merge/revert is not driven here: `detectSequencerState` is pure over
// injectable `exists`/`readFile`/`resolves` predicates specifically so each filesystem shape —
// including a forged one — can be asserted without mutating a real repository's git dir. The
// direct-entry block's own `git rev-parse --git-dir` resolution is exercised only by actually
// running a hook under a real sequencer state, which this suite does not attempt.

const SHA = "a".repeat(40);
const OTHER_SHA = "b".repeat(40);
const alwaysResolves = () => true;
const neverResolves = () => false;

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

test("resolveSkipState reports the detected state when the git-dir lookup succeeds", () => {
  const execFile = () => "/repo/.git\n";
  const { exists, readFile } = fsFrom({ "/repo/.git/MERGE_HEAD": SHA });
  const result = resolveSkipState({ execFile, exists, readFile, resolves: alwaysResolves });
  expect(result).toEqual({ determined: true, state: "merge" });
});

test("resolveSkipState reports the ordinary no-sequencer case as determined", () => {
  const execFile = () => "/repo/.git\n";
  const result = resolveSkipState({ execFile, exists: () => false, resolves: neverResolves });
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
