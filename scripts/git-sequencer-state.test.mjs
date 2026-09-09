import { test, expect } from "vitest";
import { detectSequencerState } from "./git-sequencer-state.mjs";

// A real rebase/cherry-pick/merge/revert is not driven here: `detectSequencerState` is pure over
// an injectable `exists` predicate specifically so each filesystem shape can be asserted without
// mutating a real repository's git dir. The direct-entry block's own `git rev-parse --git-dir`
// resolution is exercised only by actually running a hook under a real sequencer state, which
// this suite does not attempt.

test("reports no sequencer state when no marker is present", () => {
  expect(detectSequencerState("/repo/.git", () => false)).toBe(null);
});

test("detects an interactive/merge rebase via rebase-merge/", () => {
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/rebase-merge";
  expect(detectSequencerState("/repo/.git", exists)).toBe("rebase");
});

test("detects an am-style rebase via rebase-apply/", () => {
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/rebase-apply";
  expect(detectSequencerState("/repo/.git", exists)).toBe("rebase");
});

test("detects an in-progress cherry-pick via CHERRY_PICK_HEAD", () => {
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/CHERRY_PICK_HEAD";
  expect(detectSequencerState("/repo/.git", exists)).toBe("cherry-pick");
});

test("detects an in-progress merge via MERGE_HEAD", () => {
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/MERGE_HEAD";
  expect(detectSequencerState("/repo/.git", exists)).toBe("merge");
});

test("detects an in-progress revert via REVERT_HEAD", () => {
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/REVERT_HEAD";
  expect(detectSequencerState("/repo/.git", exists)).toBe("revert");
});

test("resolves markers under the given git dir, not a hardcoded .git", () => {
  // A worktree's git dir is a file pointing elsewhere (e.g. `.git/worktrees/<name>`); the
  // function must join against whatever path it is given, never assume the caller's cwd.
  const exists = (p) => p.replace(/\\/g, "/") === "/repo/.git/worktrees/feature/MERGE_HEAD";
  expect(detectSequencerState("/repo/.git/worktrees/feature", exists)).toBe("merge");
});
