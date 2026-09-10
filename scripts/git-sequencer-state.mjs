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
// full reasoning.
//
// HONEST LIMIT: git's own sequencer state is filesystem-trust-based, so nothing built on top of
// it can be absolute either. Each marker below is validated for shape and — for the `*_HEAD`
// files — resolved as a real object in this repository, which rules out a bare `touch` or an
// empty directory triggering the skip casually or by accident. It does not rule out a deliberate,
// informed forgery: writing an actual, resolvable commit id into the right path is still writing
// to the working tree, and no detector reading only the working tree can distinguish that from
// git's own write. What the validation buys is that the skip requires reproducing the SHAPE of a
// real sequencer state, not just its filename.

import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { runGit } from "./lib/run-git.mjs";

// A git object id: 40 hex characters in a SHA-1 repository, 64 in a SHA-256 one. Shape alone is
// not proof — `resolvesObject` below additionally confirms the id names a real object here — but
// a marker whose content fails even this shape check was not written by git.
const OBJECT_ID_RE = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i;

/**
 * Reads `path` and returns its trimmed content, or `null` if it cannot be read. Never throws —
 * a marker file that cannot be read is the same as one that is not a valid marker.
 */
function readTrimmed(path, readFile) {
  try {
    return readFile(path, "utf8").trim();
  } catch {
    return null;
  }
}

/**
 * A `*_HEAD`-style marker: valid only when the file exists, its content has the shape of a git
 * object id, AND that id resolves to a real object in this repository (via `resolves`). Rules out
 * both an empty/missing file and a file containing arbitrary non-git text with the right name.
 */
function headMarkerCheck(relativePath) {
  return (gitDir, { exists, readFile, resolves }) => {
    const p = join(gitDir, relativePath);
    if (!exists(p)) return false;
    const content = readTrimmed(p, readFile);
    return content !== null && OBJECT_ID_RE.test(content) && resolves(content);
  };
}

/**
 * A rebase-state directory: valid only when the directory, its own distinguishing marker file,
 * AND an `onto` file resolving to a real object all exist. `onto` is git's own object-id-bearing
 * file in both rebase backends (confirmed by driving a real conflicted rebase under each and
 * inspecting `.git/rebase-merge/` and `.git/rebase-apply/`), so it gets the same resolution check
 * as a `*_HEAD` file. `markerFile` is what distinguishes a genuine rebase from a directory an
 * unrelated process created: `head-name` for `rebase-merge` (present only for interactive/
 * merge-backend rebases — this directory is never used for anything else), and `rebasing` for
 * `rebase-apply`, which git itself uses to distinguish a rebase from an in-progress `git am` that
 * reuses the same directory (documented in git's own `git-prompt.sh`, the closest thing git ships
 * to a specification of this contract: `git am` writes `applying` instead).
 */
function rebaseDirCheck(dirName, markerFile) {
  return (gitDir, { exists, readFile, resolves }) => {
    const dir = join(gitDir, dirName);
    // No separate directory-existence check: `markerFile` and `onto` existing already implies
    // the directory does, and testing it separately would just be a second, redundant way for a
    // forger to satisfy half the condition (an empty directory) without the other half.
    if (!exists(join(dir, markerFile))) return false;
    const ontoContent = readTrimmed(join(dir, "onto"), readFile);
    return ontoContent !== null && OBJECT_ID_RE.test(ontoContent) && resolves(ontoContent);
  };
}

/**
 * The five sequencer states this module can detect, each with its own validated check. First
 * match wins; git never leaves more than one active, but a stale leftover after a failed
 * operation is possible, and reporting any one of them is enough to justify the skip.
 */
export const SEQUENCER_MARKERS = [
  { state: "rebase", check: rebaseDirCheck("rebase-merge", "head-name") },
  { state: "rebase", check: rebaseDirCheck("rebase-apply", "rebasing") },
  { state: "cherry-pick", check: headMarkerCheck("CHERRY_PICK_HEAD") },
  { state: "merge", check: headMarkerCheck("MERGE_HEAD") },
  { state: "revert", check: headMarkerCheck("REVERT_HEAD") },
];

/**
 * Builds the default object-resolution predicate for `gitDir`: an id "resolves" when `git
 * cat-file -e <id>` succeeds against that git dir. Routed through the shared `runGit`, so a
 * failure to run git at all (as opposed to the id genuinely not resolving) is reported the same
 * way as any other `runGit` failure — never a raw exception — and simply counts as "does not
 * resolve", which is the safe direction: an unresolvable id means the marker is not valid, and an
 * invalid marker means this state is not detected, which falls through toward running the tier.
 */
function defaultResolves(gitDir, execFile) {
  return (id) =>
    runGit(["--git-dir", gitDir, "cat-file", "-e", id], "the sequencer marker object", {
      execFile,
    }).ok;
}

/**
 * The sequencer state git has put `gitDir` into, or `null` when none applies. Pure over its
 * inputs — `gitDir` plus injectable `exists`/`readFile`/`resolves`/`execFile` — so it is testable
 * without driving a real rebase or touching a real repository's object store.
 *
 * @param {string} gitDir - the repository's git dir, as reported by `git rev-parse --git-dir`.
 * @param {{
 *   exists?: (path: string) => boolean,
 *   readFile?: (path: string, enc: string) => string,
 *   resolves?: (id: string) => boolean,
 *   execFile?: typeof execFileSync,
 * }} [deps] - injectable for tests.
 * @returns {string | null} the detected state name, or `null` when no valid sequencer marker is
 *   present.
 */
export function detectSequencerState(gitDir, deps = {}) {
  const exists = deps.exists ?? existsSync;
  const readFile = deps.readFile ?? readFileSync;
  const resolves = deps.resolves ?? defaultResolves(gitDir, deps.execFile);
  const merged = { exists, readFile, resolves };
  for (const marker of SEQUENCER_MARKERS) {
    if (marker.check(gitDir, merged)) return marker.state;
  }
  return null;
}

/**
 * Resolves what `pre-commit` should do: run the tier, or skip it for a named sequencer state.
 * Isolates the one step in this module that can fail for reasons outside its control (the `git
 * rev-parse --git-dir` call, routed through the shared `runGit` — see `scripts/lib/run-git.mjs`)
 * from `detectSequencerState`'s own logic, so a git failure here cannot silently reach the caller
 * as a skip.
 *
 * FAILS TOWARD RUNNING THE TIER: when the git-dir lookup itself fails, the sequencer state is
 * UNKNOWN, not "no sequencer state" — and an unknown state that skips is a silent hole in the
 * gate, while an unknown state that runs the tier costs ~70s and nothing else. So `determined:
 * false` is the caller's signal to run the tier, exactly like `detectSequencerState` returning
 * `null` for "no valid marker found" — same as the derived mtime exemption elsewhere in this
 * project failing toward stricter when its own manifest entry is absent. This is THIS caller's
 * choice of safe direction, not `runGit`'s — the shared helper only reports failure, never decides
 * what it means.
 *
 * @param {{
 *   execFile?: typeof execFileSync,
 *   exists?: (path: string) => boolean,
 *   readFile?: (path: string, enc: string) => string,
 *   resolves?: (id: string) => boolean,
 * }} [deps] - injectable for tests: `execFile` to make the git-dir lookup fail (or to drive the
 *   default object-resolution check), `exists`/`readFile`/`resolves` to drive
 *   `detectSequencerState` directly.
 * @returns {{ determined: true, state: string | null } | { determined: false, reason: string }}
 */
export function resolveSkipState({ execFile = execFileSync, exists, readFile, resolves } = {}) {
  const gitDirResult = runGit(["rev-parse", "--git-dir"], "the sequencer state", { execFile });
  if (!gitDirResult.ok) return { determined: false, reason: gitDirResult.message };
  return {
    determined: true,
    state: detectSequencerState(gitDirResult.stdout, { exists, readFile, resolves, execFile }),
  };
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
