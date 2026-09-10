// Detects whether git has this repository mid a sequencer operation (rebase, cherry-pick,
// revert, or merge) with genuine remaining work — read from the git dir's own filesystem
// markers, which are git's own authoritative signal for this, not a `git status` porcelain guess
// that could drift from it.
//
// `pre-commit` uses this to skip the commit tier while one of these operations is genuinely
// unfinished: a rebase/cherry-pick/revert with steps still queued in its own todo file, or any of
// the five states sitting on an unresolved conflict in the index. A sequencer marker present with
// NEITHER of those — a clean `git merge --no-commit --no-ff` about to be concluded by an ordinary
// `git commit`, or a cherry-pick/revert whose conflict has already been resolved and staged — is
// not remaining work; the commit about to happen is the same kind of ordinary, reviewable commit
// this tier exists to gate, and it gates normally. See `pre-commit`'s own comment for the reasons
// a genuinely-unfinished operation is exempted at all.
//
// HONEST LIMIT: git's own sequencer state is filesystem-trust-based, so nothing built on top of
// it can be absolute either. Every `*_HEAD` marker, the rebase directories' own `onto` file, and
// the remaining-work signals below (a todo file's queued lines, `rebase-apply`'s `next`/`last`
// counters, the index's unmerged entries) are all read from paths and values git itself writes —
// which rules out a bare `touch` or an empty directory triggering a skip. It does NOT rule out a
// deliberate, informed forgery: writing an actual, resolvable commit id and a plausible todo/
// counter file into the right paths is still writing to the working tree, and no detector reading
// only the working tree can distinguish that from git's own write. Nor is the skip rare or exotic
// under ORDINARY use: any real conflict left unresolved for a while — someone walks away mid
// rebase, or leaves a conflicted `git merge --no-commit` open — reaches it with no forgery at all.
// What the validation buys is that the skip requires reproducing the SHAPE of a real sequencer
// state that ACTUALLY has work left, not just its filename; it is not a claim that the skip is
// hard to reach by accident.

import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { runGit } from "./lib/run-git.mjs";

// A git object id: 40 hex characters in a SHA-1 repository, 64 in a SHA-256 one. Shape alone is
// not proof — `isResolvableObjectId` below additionally confirms the id names a real object here
// — but a line failing even this shape check was not written by git. Tested per LINE, never
// against a whole multi-line file at once: an octopus merge (three or more parents) writes
// multiple newline-separated object ids into `MERGE_HEAD`, and anchoring this against the whole
// trimmed content would fail the shape check on every real octopus merge.
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

/** True when `line` has the shape of a git object id AND resolves to a real object via `resolves`. */
function isResolvableObjectId(line, resolves) {
  return OBJECT_ID_RE.test(line) && resolves(line);
}

/**
 * A `*_HEAD`-style marker: valid only when the file exists and EVERY non-blank line resolves to a
 * real object in this repository (via `resolves`). Checked per line, not as one block, so a
 * multi-parent `MERGE_HEAD` (an octopus merge) is covered — each parent id still has to
 * independently pass the same shape-and-resolution bar a single-parent id does, so the forgery
 * bar is identical to the single-line case, just applied per line. Rules out an empty/missing
 * file, a file containing arbitrary non-git text, and a file with even one line that does not
 * resolve.
 */
function headMarkerCheck(relativePath) {
  return (gitDir, { exists, readFile, resolves }) => {
    const p = join(gitDir, relativePath);
    if (!exists(p)) return false;
    const content = readTrimmed(p, readFile);
    if (content === null) return false;
    const lines = content.split("\n").map((line) => line.trim()).filter((line) => line !== "");
    return lines.length > 0 && lines.every((line) => isResolvableObjectId(line, resolves));
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
    // `onto` is always a single object id — no octopus-rebase concept exists — so this stays a
    // one-line check rather than `headMarkerCheck`'s per-line loop.
    const ontoContent = readTrimmed(join(dir, "onto"), readFile);
    return ontoContent !== null && isResolvableObjectId(ontoContent, resolves);
  };
}

/**
 * The non-comment, non-blank lines of a todo-style file's content — a queued-step count for
 * `git-rebase-todo` (`rebase-merge`) and `sequencer/todo` (multi-commit cherry-pick/revert).
 * Plain non-interactive runs write no comment header (confirmed by driving a real conflicted
 * rebase and a real conflicted multi-commit cherry-pick and reading the raw file), but the filter
 * is applied regardless: git's interactive form does write one, and a line beginning with `#` is
 * never a queued step under either form.
 */
function nonEmptyTodoLines(content) {
  return content
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"));
}

/**
 * Whether `gitDir`/`relativePath`'s content (read via `readFile`) has at least one queued line
 * beyond the given `skip` count. `readTrimmed` returning `null` (missing/unreadable file) counts
 * as no queued lines, the same fail-toward-running-the-tier direction the rest of this module
 * uses for an unreadable marker.
 */
function hasQueuedLines(gitDir, relativePath, readFile, skip = 0) {
  const content = readTrimmed(join(gitDir, relativePath), readFile);
  return content !== null && nonEmptyTodoLines(content).length > skip;
}

/**
 * Reads `gitDir`/`relativePath` as a base-10 integer, or `null` when the file is missing,
 * unreadable, or not a plain integer.
 */
function readIntFile(gitDir, relativePath, readFile) {
  const content = readTrimmed(join(gitDir, relativePath), readFile);
  if (content === null || !/^\d+$/.test(content)) return null;
  return Number.parseInt(content, 10);
}

/**
 * `rebase-merge` (interactive/merge-backend rebase): remaining work is a queued `pick`/etc. line
 * in `git-rebase-todo`, OR an unresolved conflict in the index. Measured against a real conflicted
 * rebase: the step currently stopped on a conflict is NOT itself listed in `git-rebase-todo` (git
 * moves it to `done` as soon as it starts applying), so at the LAST step — conflict resolved,
 * staged, about to be concluded by an ordinary `git rebase --continue`-driven commit — the todo
 * file is empty and this correctly reports no remaining work, the rebase analogue of a clean
 * `git merge --no-commit` sitting with nothing left to resolve.
 */
function rebaseMergeRemainingWork(gitDir, { readFile, hasConflicts }) {
  return hasQueuedLines(gitDir, "rebase-merge/git-rebase-todo", readFile) || hasConflicts();
}

/**
 * `rebase-apply` (the apply/`am` backend, e.g. `git rebase --apply`): remaining work is
 * `next < last` (a further numbered patch queued beyond the one currently being applied), OR an
 * unresolved conflict in the index. Measured against a real conflicted `--apply` rebase: at the
 * FINAL patch — conflict resolved, staged, about to be concluded — `next` equals `last`, the same
 * "nothing left" shape `rebase-merge`'s empty todo reports at its own last step. A missing or
 * non-numeric counter (`readIntFile` returning `null`) is treated as no queued patches, falling
 * back to the conflict check alone — the same fail-toward-running-the-tier direction as an
 * unreadable todo file above.
 */
function rebaseApplyRemainingWork(gitDir, { readFile, hasConflicts }) {
  const next = readIntFile(gitDir, "rebase-apply/next", readFile);
  const last = readIntFile(gitDir, "rebase-apply/last", readFile);
  const patchesQueued = next !== null && last !== null && next < last;
  return patchesQueued || hasConflicts();
}

/**
 * `CHERRY_PICK_HEAD`/`REVERT_HEAD`: remaining work is a queued entry in `sequencer/todo` BEYOND
 * the current one, OR an unresolved conflict in the index. A single-commit cherry-pick/revert
 * (the common case) writes no `sequencer/todo` at all — confirmed empirically — so `skip: 1` only
 * matters for a multi-commit `git cherry-pick a b c`/`git revert a b c`, where the current,
 * already-stopped-on entry is always still listed as `sequencer/todo`'s FIRST line even after its
 * own conflict is resolved and staged (git does not rewrite the file until `--continue` actually
 * advances), so a bare non-empty check would report "remaining" forever. Measured against a real
 * multi-commit cherry-pick: with the first conflict resolved and staged but nothing beyond it
 * queued, `sequencer/todo` holds exactly one line and this correctly reports no remaining work —
 * the cherry-pick/revert analogue of a clean merge sitting with nothing left to resolve.
 */
function sequencerRemainingWork(gitDir, { readFile, hasConflicts }) {
  return hasQueuedLines(gitDir, "sequencer/todo", readFile, 1) || hasConflicts();
}

/**
 * `MERGE_HEAD`: a merge has no todo/counter file of its own — one merge is one commit, never a
 * queued sequence — so remaining work is purely an unresolved conflict in the index. A clean
 * `git merge --no-commit --no-ff` sitting with nothing to resolve reports no remaining work: the
 * commit about to conclude it is the ordinary merge commit this tier exists to gate.
 */
function conflictOnlyRemainingWork(_gitDir, { hasConflicts }) {
  return hasConflicts();
}

/**
 * The five sequencer states this module can detect, each with its own validated `check` (shape +
 * object resolution, forgery-resistant but not forgery-proof — see the HONEST LIMIT above) and its
 * own `remainingWork` predicate (whether that state currently has genuine work left, per the
 * functions above). First match wins; git never leaves more than one active, but a stale leftover
 * after an abandoned operation is possible — `rebaseDirCheck`'s two entries are the SAME state
 * name under two different on-disk backends with two different remaining-work signals (a todo
 * file's queued lines vs. a numbered-patch counter), so `check` alone decides which backend
 * matched before `remainingWork` is asked anything.
 */
export const SEQUENCER_MARKERS = [
  {
    state: "rebase",
    check: rebaseDirCheck("rebase-merge", "head-name"),
    remainingWork: rebaseMergeRemainingWork,
  },
  {
    state: "rebase",
    check: rebaseDirCheck("rebase-apply", "rebasing"),
    remainingWork: rebaseApplyRemainingWork,
  },
  {
    state: "cherry-pick",
    check: headMarkerCheck("CHERRY_PICK_HEAD"),
    remainingWork: sequencerRemainingWork,
  },
  { state: "merge", check: headMarkerCheck("MERGE_HEAD"), remainingWork: conflictOnlyRemainingWork },
  {
    state: "revert",
    check: headMarkerCheck("REVERT_HEAD"),
    remainingWork: sequencerRemainingWork,
  },
];

/**
 * A command hint per state, named in the loud skip message so a stale sequencer state is visible
 * rather than silently exempting every later commit — see the direct-entry block below.
 */
export const CONCLUDE_HINT = {
  rebase: "run `git rebase --continue` after resolving conflicts, or `git rebase --abort` to cancel it",
  "cherry-pick":
    "run `git cherry-pick --continue` after resolving conflicts, or `git cherry-pick --abort` to cancel it",
  merge: "run `git commit` to conclude it, or `git merge --abort` to cancel it",
  revert: "run `git revert --continue` after resolving conflicts, or `git revert --abort` to cancel it",
};

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
 * Builds the default unresolved-conflict predicate for `gitDir`: true when the index holds any
 * unmerged ("stage > 0") entry, read via `git ls-files -u` against `gitDir` alone — an unmerged
 * entry lives in the index, so no working-tree/`--work-tree` argument is needed to see it
 * (verified against a real conflicted merge/rebase/cherry-pick/revert from OUTSIDE the repo's own
 * cwd). Routed through `runGit`, so a git-tooling failure here counts as "no conflict found",
 * the same fail-toward-running-the-tier direction `defaultResolves` uses for an unresolvable id.
 */
function defaultHasConflicts(gitDir, execFile) {
  return () => {
    const result = runGit(["--git-dir", gitDir, "ls-files", "-u"], "the unresolved-conflict list", {
      execFile,
    });
    return result.ok && result.stdout !== "";
  };
}

/**
 * The sequencer state git has put `gitDir` into, plus whether it currently has genuine remaining
 * work, or `null` when no marker applies. Pure over its inputs — `gitDir` plus injectable
 * `exists`/`readFile`/`resolves`/`hasConflicts`/`execFile` — so it is testable without driving a
 * real rebase or touching a real repository's object store. A single pass over `SEQUENCER_MARKERS`
 * rather than two (one to pick the state, one to ask about remaining work): the winning marker's
 * OWN `remainingWork` is invoked directly, so a `rebase-merge` match can never be scored against
 * `rebase-apply`'s counter-based check or vice versa.
 *
 * @param {string} gitDir - the repository's git dir, as reported by `git rev-parse --git-dir`.
 * @param {{
 *   exists?: (path: string) => boolean,
 *   readFile?: (path: string, enc: string) => string,
 *   resolves?: (id: string) => boolean,
 *   hasConflicts?: () => boolean,
 *   execFile?: typeof execFileSync,
 * }} [deps] - injectable for tests.
 * @returns {{ state: string, remainingWork: boolean } | null}
 */
export function detectSequencer(gitDir, deps = {}) {
  const exists = deps.exists ?? existsSync;
  const readFile = deps.readFile ?? readFileSync;
  const resolves = deps.resolves ?? defaultResolves(gitDir, deps.execFile);
  const hasConflicts = deps.hasConflicts ?? defaultHasConflicts(gitDir, deps.execFile);
  const merged = { exists, readFile, resolves, hasConflicts };
  for (const marker of SEQUENCER_MARKERS) {
    if (marker.check(gitDir, merged)) {
      return { state: marker.state, remainingWork: marker.remainingWork(gitDir, merged) };
    }
  }
  return null;
}

/**
 * The sequencer state git has put `gitDir` into, or `null` when none applies — regardless of
 * whether that state currently has remaining work. A thin wrapper over `detectSequencer` kept for
 * callers that only need to know WHICH operation is in progress, not whether it should exempt a
 * commit.
 *
 * @param {string} gitDir - the repository's git dir, as reported by `git rev-parse --git-dir`.
 * @param {Parameters<typeof detectSequencer>[1]} [deps] - injectable for tests.
 * @returns {string | null} the detected state name, or `null` when no valid sequencer marker is
 *   present.
 */
export function detectSequencerState(gitDir, deps = {}) {
  return detectSequencer(gitDir, deps)?.state ?? null;
}

/**
 * Resolves what `pre-commit` should do: run the tier, or skip it because a sequencer state with
 * genuine remaining work is in progress. Isolates the one step in this module that can fail for
 * reasons outside its control (the `git rev-parse --git-dir` call, routed through the shared
 * `runGit` — see `scripts/lib/run-git.mjs`) from `detectSequencer`'s own logic, so a git failure
 * here cannot silently reach the caller as a skip.
 *
 * FAILS TOWARD RUNNING THE TIER, at two separate points: when the git-dir lookup itself fails, the
 * sequencer state is UNKNOWN, not "no sequencer state" — `determined: false` is the caller's
 * signal to run the tier regardless, exactly like `state: null` below for "a marker is present but
 * it has no remaining work" or "no valid marker found" at all. An unknown state that skips is a
 * silent hole in the gate, while an unknown state that runs the tier costs ~70s and nothing else.
 * This is THIS caller's choice of safe direction, not `runGit`'s — the shared helper only reports
 * failure, never decides what it means.
 *
 * @param {{
 *   execFile?: typeof execFileSync,
 *   exists?: (path: string) => boolean,
 *   readFile?: (path: string, enc: string) => string,
 *   resolves?: (id: string) => boolean,
 *   hasConflicts?: () => boolean,
 * }} [deps] - injectable for tests: `execFile` to make the git-dir lookup fail (or to drive the
 *   default object-resolution/conflict checks), `exists`/`readFile`/`resolves`/`hasConflicts` to
 *   drive `detectSequencer` directly.
 * @returns {{ determined: true, state: string | null } | { determined: false, reason: string }}
 */
export function resolveSkipState({ execFile = execFileSync, exists, readFile, resolves, hasConflicts } = {}) {
  const gitDirResult = runGit(["rev-parse", "--git-dir"], "the sequencer state", { execFile });
  if (!gitDirResult.ok) return { determined: false, reason: gitDirResult.message };
  const detected = detectSequencer(gitDirResult.stdout, {
    exists,
    readFile,
    resolves,
    hasConflicts,
    execFile,
  });
  return { determined: true, state: detected?.remainingWork ? detected.state : null };
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
  // non-empty line means; printing nothing on no-detection (or on a marker with no remaining
  // work) lets a shell `if [ -n "$x" ]` read it directly.
  if (result.state) {
    // Loud and specific rather than `pre-commit`'s previous one-line note: a sequencer state left
    // in place by an abandoned operation still exempts every later commit for as long as it sits
    // there, so the message names the state AND the exact command to conclude it, on stderr,
    // every single time the skip fires — never only once.
    console.error(
      `git-sequencer-state: skipping the commit tier — a ${result.state} is in progress with unresolved work. ` +
        `Conclude it before it goes stale: ${CONCLUDE_HINT[result.state]}.`,
    );
    console.log(result.state);
  }
  process.exit(0);
}
