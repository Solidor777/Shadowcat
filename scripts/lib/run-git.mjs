// The single guarded entry point for shelling out to git: every caller in this repo that needs to
// run a git command routes through this, so a failing invocation never reaches an operator as a
// raw Node stack trace, and no caller inherits a git-repository-pointing environment variable it
// never asked for.
//
// Deliberately does NOT decide what a failure means for the caller. Different callers need
// mutually incompatible safe directions for the exact same failure — degrade to a warning and keep
// going, run a fallback path rather than skip on an unknown state, refuse an operation outright —
// so this returns a result the caller inspects and reacts to, never a decision this module makes
// on the caller's behalf.

import { execFileSync } from "node:child_process";

// Environment variables git itself defines as repository/index/object-store pointers (see `git
// help environment`). Git exports several of these into every child process it spawns — a hook or
// a script invoked from inside a `git commit` inherits `GIT_INDEX_FILE` pointing at the commit's
// in-flight temporary index rather than the repository's real one, and `GIT_PREFIX`/`GIT_DIR`
// carry the same hazard for the working-tree root and the `.git` directory. A `-C <dir>` argument
// does not override any of these: git resolves the pointer variables before it looks at `-C`, so
// an inherited `GIT_INDEX_FILE` silently redirects a command aimed at an unrelated repository back
// onto the parent process's own index. Scrubbing them is therefore the default, not opt-in: no
// caller of this module wants git's target repository decided by whatever process happened to
// launch it. `GIT_EXEC_PATH` is deliberately excluded — git uses it to locate its own
// subcommands, and clearing it can break git itself rather than merely re-target it.
//
// This is the ONE copy of the list. A test fixture that drives git directly reaches it through
// `scrubGitEnv` rather than restating it: a second copy is a second place for the two to disagree
// on which variables count, with nothing to report the disagreement.
export const GIT_ENV_VARS_TO_SCRUB = [
  "GIT_DIR",
  "GIT_INDEX_FILE",
  "GIT_WORK_TREE",
  "GIT_OBJECT_DIRECTORY",
  "GIT_ALTERNATE_OBJECT_DIRECTORIES",
  "GIT_COMMON_DIR",
  "GIT_NAMESPACE",
  "GIT_PREFIX",
  "GIT_CEILING_DIRECTORIES",
  "GIT_INDEX_VERSION",
];

/**
 * A copy of `env` with every variable in `GIT_ENV_VARS_TO_SCRUB` removed — the environment every
 * git child process this repo spawns starts from. `runGit` applies it on every call; a test
 * fixture that drives git directly against a scratch repository passes it as that call's `env`,
 * because the test process itself runs inside `git commit`'s hook environment and inherits the
 * same pointer variables, so an unscrubbed fixture `git add` lands on the enclosing commit's index.
 *
 * @param {NodeJS.ProcessEnv} [env] - the environment to copy; defaults to `process.env`.
 * @returns {NodeJS.ProcessEnv} a fresh object; `env` itself is never mutated.
 */
export function scrubGitEnv(env = process.env) {
  const scrubbed = { ...env };
  for (const key of GIT_ENV_VARS_TO_SCRUB) delete scrubbed[key];
  return scrubbed;
}

/**
 * Runs `git <args>` and returns its trimmed stdout, or a legible failure describing what the
 * caller could not determine — never a raw exception. `what` is folded into the message in the
 * caller's own words (e.g. "the repository root", "the sequencer state"), so the failure reads as
 * a sentence about what could not be established rather than a bare shell error.
 *
 * By default the child process's environment has every variable in `GIT_ENV_VARS_TO_SCRUB`
 * removed, so an inherited `GIT_INDEX_FILE`/`GIT_DIR`/etc. from an enclosing `git commit` (or any
 * other git invocation) can never redirect this call onto the wrong repository. Pass `env` to
 * supply a specific environment instead of inheriting `process.env` — it is scrubbed the same way.
 *
 * @param {string[]} args - argv passed to `git`, e.g. `["rev-parse", "--git-dir"]`.
 * @param {string} what - a short description of what this call was trying to determine.
 * @param {{ execFile?: typeof execFileSync, env?: NodeJS.ProcessEnv }} [deps] - injectable for
 *   tests; `execFile` defaults to the real `execFileSync`, `env` defaults to `process.env`.
 * @returns {{ ok: true, stdout: string } | { ok: false, message: string }}
 */
export function runGit(args, what, { execFile = execFileSync, env = process.env } = {}) {
  try {
    const stdout = execFile("git", args, { encoding: "utf8", env: scrubGitEnv(env) }).trim();
    return { ok: true, stdout };
  } catch (err) {
    return {
      ok: false,
      message: `could not determine ${what} (\`git ${args.join(" ")}\` failed: ${err.message})`,
    };
  }
}
