// Detects whether git has this repository mid a sequencer operation (rebase, cherry-pick,
// revert, or merge) — read from the git dir's own filesystem markers, which are git's own
// authoritative signal for this, not a `git status` porcelain guess that could drift from it.
//
// `pre-commit` uses this to skip the commit tier while one of these operations is running: git
// replays each commit through the hook once per commit, multiplying the tier by the commit
// count, which can outrun an agent's shell cap mid-sequence with no permitted escape. None of the
// replayed commits is ever pushed in isolation — the sequencer produces ordinary commits inside
// one operation, and `pre-push`'s tree-keyed receipt still gates the eventual push and
// independently refuses on a dirty tree — so skipping here removes redundant intermediate work
// without letting anything unverified reach the remote. See `pre-commit`'s own comment for the
// full reasoning; this module is deliberately just the detector, with no flag or environment
// variable that could trigger the same skip outside a real sequencer state.

import { existsSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

/**
 * The five filesystem markers git itself uses for an in-progress sequencer operation, each
 * resolved relative to the git dir (never a literal `.git` — a worktree's git dir is a file
 * pointing elsewhere, so callers must resolve it via `git rev-parse --git-dir`). First match
 * wins; git never leaves more than one active, but a stale leftover after a failed operation is
 * possible, and reporting any one of them is enough to justify the skip.
 */
export const SEQUENCER_MARKERS = [
  { state: "rebase", relativePath: "rebase-merge" },
  { state: "rebase", relativePath: "rebase-apply" },
  { state: "cherry-pick", relativePath: "CHERRY_PICK_HEAD" },
  { state: "merge", relativePath: "MERGE_HEAD" },
  { state: "revert", relativePath: "REVERT_HEAD" },
];

/**
 * The sequencer state git has put `gitDir` into, or `null` when none applies. Pure over its
 * inputs — `gitDir` plus an injectable `exists` predicate — so it is testable without driving a
 * real rebase.
 *
 * @param {string} gitDir - the repository's git dir, as reported by `git rev-parse --git-dir`.
 * @param {(path: string) => boolean} [exists] - existence check, injectable for tests.
 * @returns {string | null} the detected state name, or `null` when no sequencer marker is present.
 */
export function detectSequencerState(gitDir, exists = existsSync) {
  for (const marker of SEQUENCER_MARKERS) {
    if (exists(join(gitDir, marker.relativePath))) return marker.state;
  }
  return null;
}

/**
 * Resolves what `pre-commit` should do: run the tier, or skip it for a named sequencer state.
 * Isolates the one step in this module that can fail for reasons outside its control (the `git
 * rev-parse --git-dir` call) from `detectSequencerState`'s pure logic, so a git failure here
 * cannot silently reach the caller as a skip.
 *
 * FAILS TOWARD RUNNING THE TIER: when the git-dir lookup itself throws, the sequencer state is
 * UNKNOWN, not "no sequencer state" — and an unknown state that skips is a silent hole in the
 * gate, while an unknown state that runs the tier costs ~70s and nothing else. So `determined:
 * false` is the caller's signal to run the tier, exactly like `detectSequencerState` returning
 * `null` for "no marker found" — same as the derived mtime exemption elsewhere in this project
 * failing toward stricter when its own manifest entry is absent.
 *
 * @param {{ execFile?: typeof execFileSync, exists?: (path: string) => boolean }} [deps] -
 *   injectable for tests: `execFile` to make the git-dir lookup throw, `exists` to drive
 *   `detectSequencerState`.
 * @returns {{ determined: true, state: string | null } | { determined: false, reason: string }}
 */
export function resolveSkipState({ execFile = execFileSync, exists = existsSync } = {}) {
  let gitDir;
  try {
    gitDir = execFile("git", ["rev-parse", "--git-dir"], { encoding: "utf8" }).trim();
  } catch (err) {
    return { determined: false, reason: err.message };
  }
  return { determined: true, state: detectSequencerState(gitDir, exists) };
}

if (isDirectEntry(import.meta.url)) {
  const result = resolveSkipState();
  if (!result.determined) {
    // Prints to stderr, prints NOTHING to stdout, and still exits 0: `pre-commit` reads stdout
    // to decide whether to skip, so an empty stdout here means "run the tier" even though the
    // process itself exits cleanly rather than propagating a raw git failure to the operator.
    console.error(
      `git-sequencer-state: could not determine sequencer state (${result.reason}) — running the tier anyway.`,
    );
    process.exit(0);
  }
  // Always exits 0: this is a detector, not a gate. The caller (`pre-commit`) decides what a
  // non-empty line means; printing nothing on no-detection lets a shell `if [ -n "$x" ]` read it
  // directly.
  if (result.state) console.log(result.state);
  process.exit(0);
}
