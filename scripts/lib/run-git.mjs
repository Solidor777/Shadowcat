// A single guarded entry point for shelling out to git, so a failing invocation never reaches an
// operator as a raw Node stack trace. This plan's three entry points — the hook installer, the
// sequencer probe, and the tier runner — each had their own unguarded `execFileSync("git", ...)`
// at an entry point, fixed one at a time across three separate rounds; three instances of one
// class means the site was never the cause, so this module is the fix instead of a fourth patch.
//
// Deliberately does NOT decide what a failure means for the caller: the installer must degrade to
// a warning and keep `pnpm install` succeeding, the sequencer probe must run the tier rather than
// skip on an unknown state, and the tier runner's `--verify-receipt` path must refuse the push —
// three mutually incompatible safe directions for the exact same failure. Collapsing them into one
// behavior here (throwing, exiting, or picking a default) would silently choose the wrong one for
// at least two of the three callers, so this returns a result the caller inspects and reacts to,
// never a decision this module makes on the caller's behalf.

import { execFileSync } from "node:child_process";

/**
 * Runs `git <args>` and returns its trimmed stdout, or a legible failure describing what the
 * caller could not determine — never a raw exception. `what` is folded into the message in the
 * caller's own words (e.g. "the repository root", "the sequencer state"), so the failure reads as
 * a sentence about what could not be established rather than a bare shell error.
 *
 * @param {string[]} args - argv passed to `git`, e.g. `["rev-parse", "--git-dir"]`.
 * @param {string} what - a short description of what this call was trying to determine.
 * @param {{ execFile?: typeof execFileSync }} [deps] - injectable for tests; defaults to the real
 *   `execFileSync`.
 * @returns {{ ok: true, stdout: string } | { ok: false, message: string }}
 */
export function runGit(args, what, { execFile = execFileSync } = {}) {
  try {
    const stdout = execFile("git", args, { encoding: "utf8" }).trim();
    return { ok: true, stdout };
  } catch (err) {
    return {
      ok: false,
      message: `could not determine ${what} (\`git ${args.join(" ")}\` failed: ${err.message})`,
    };
  }
}
