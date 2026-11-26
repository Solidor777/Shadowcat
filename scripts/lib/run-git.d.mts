/** Types of `run-git.mjs`, so TypeScript callers (the e2e click-rule test) get its signatures. */

/** The outcome of a git invocation: its trimmed stdout, or a sentence saying what could not be determined. */
export type GitResult = { ok: true; stdout: string } | { ok: false; message: string };

/** The environment variables git defines as repository pointers, removed from every child environment. */
export const GIT_ENV_VARS_TO_SCRUB: string[];

/**
 * Copies `env` without the git repository-pointer variables.
 * @param env The environment to copy; defaults to `process.env`.
 * @returns A fresh object.
 * @example
 * ```ts
 * const env = scrubGitEnv({ GIT_DIR: "x", PATH: "y" }); // => { PATH: "y" }
 * ```
 */
export function scrubGitEnv(env?: NodeJS.ProcessEnv): NodeJS.ProcessEnv;

/**
 * Runs `git <args>` with the pointer variables scrubbed.
 * @param args The argv passed to `git`.
 * @param what A short description of what the call determines, used in a failure message.
 * @param deps Injectable `execFile` and `env`.
 * @returns The trimmed stdout, or a legible failure.
 * @example
 * ```ts
 * const r = runGit(["rev-parse", "--git-dir"], "the git directory");
 * ```
 */
export function runGit(
  args: string[],
  what: string,
  deps?: { execFile?: (file: string, args: string[], options: object) => string; env?: NodeJS.ProcessEnv },
): GitResult;
