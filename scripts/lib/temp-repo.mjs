// Scratch git repositories for the gate tests: a gate's corpus is whatever git lists, so a test of
// "an untracked file fails the gate" needs a real repository holding that file.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { runGit, scrubGitEnv } from "./run-git.mjs";

/** The repository root that holds the `scripts/` directory this module lives in. */
const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * Creates a scratch repository holding `files`, indexing only the paths in `tracked` (every file
 * when omitted). A path written but not indexed is an untracked file; one named by `.gitignore` is
 * an ignored file.
 *
 * @param {Record<string, string>} files - repository-relative path to file text.
 * @param {string[]} [tracked] - paths to `git add`; defaults to every key of `files`.
 * @returns {string} the repository directory.
 * @example
 * const dir = makeRepo({ "src/a.rs": "fn a() {}\n" }, []);
 */
export function makeRepo(files, tracked = Object.keys(files)) {
  const dir = mkdtempSync(join(tmpdir(), "gate-repo-"));
  const git = (args) => {
    const r = runGit(["-C", dir, ...args], "a scratch repository operation");
    if (!r.ok) throw new Error(r.message);
  };
  git(["init", "-q"]);
  for (const [path, text] of Object.entries(files)) {
    const abs = join(dir, ...path.split("/"));
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, text);
  }
  for (const path of tracked) git(["add", "--", path]);
  return dir;
}

/**
 * Runs a repo script with `dir` as its working directory and no skill checkout, so the gate sees
 * only the scratch repository.
 *
 * @param {string} dir - the working directory.
 * @param {string} script - repository-relative script path, e.g. `scripts/check-file-lines.mjs`.
 * @param {string[]} [args] - script arguments.
 * @returns {{ status: number|null, stdout: string, stderr: string }} the exit status and output.
 * @example
 * runScriptIn(makeRepo({}), "scripts/check-file-lines.mjs").status;
 */
export function runScriptIn(dir, script, args = []) {
  const r = spawnSync(process.execPath, [join(REPO_ROOT, ...script.split("/")), ...args], {
    cwd: dir,
    env: { ...scrubGitEnv(), SHADOWCAT_CODEBASE_SKILLS_DIR: join(dir, "no-skill-checkout") },
    encoding: "utf8",
  });
  return { status: r.status, stdout: r.stdout, stderr: r.stderr };
}
