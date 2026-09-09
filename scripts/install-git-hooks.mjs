// Points this worktree's core.hooksPath at the tracked hooks, so a fresh clone arms itself.
//
// Wired to `prepare`, which pnpm runs on install — the one command every clone and every new
// worktree already runs, so arming needs nobody to remember it.
//
// Scoped per worktree: sibling worktrees share one .git/config, so a repository-wide hooksPath
// would make every worktree run one checkout's hooks. extensions.worktreeConfig moves the
// setting into the per-worktree config, so each runs its own branch's.

import { chmodSync, existsSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export function hooksDir(repoRoot) {
  return join(repoRoot, "scripts", "git-hooks");
}

/** CI checks out fresh per job and pushes nothing; arming it would only slow the runner. */
export function shouldSkip(env) {
  return Boolean(env.CI || env.GITHUB_ACTIONS);
}

/** The git config invocations, in order. worktreeConfig must be enabled before --worktree works. */
export function configPlan(repoRoot) {
  return [
    { args: ["config", "extensions.worktreeConfig", "true"] },
    { args: ["config", "--worktree", "core.hooksPath", hooksDir(repoRoot)] },
  ];
}

if (isDirectEntry(import.meta.url)) {
  if (shouldSkip(process.env)) {
    console.log("install-git-hooks: skipped under CI");
    process.exit(0);
  }
  const root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
  const dir = hooksDir(root);
  if (!existsSync(dir)) {
    console.error(`install-git-hooks: ${dir} is missing`);
    process.exit(1);
  }
  for (const step of configPlan(root)) {
    execFileSync("git", step.args, { stdio: "inherit" });
  }
  // core.filemode is false on Windows checkouts, so the bit is set explicitly rather than
  // relying on the checkout to carry it.
  for (const name of readdirSync(dir)) chmodSync(join(dir, name), 0o755);
  console.log(`install-git-hooks: core.hooksPath -> ${dir}`);
}
