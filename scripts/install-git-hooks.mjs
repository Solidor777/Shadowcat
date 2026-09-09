// Points this worktree's core.hooksPath at the tracked hooks, so a fresh clone arms itself.
//
// Wired to `prepare`, which pnpm runs on install — the one command every clone and every new
// worktree already runs, so arming needs nobody to remember it.
//
// Scoped per worktree: sibling worktrees share one .git/config, so a repository-wide hooksPath
// would make every worktree run one checkout's hooks. extensions.worktreeConfig moves the
// setting into the per-worktree config, so each runs its own branch's.
// PRECONDITION: git documents that once `extensions.worktreeConfig` is active, `core.bare` and
// `core.worktree` must themselves live in the per-worktree config or a repo can behave as though
// it were bare. Neither key is set anywhere in this repo today, so enabling the extension here is
// safe; if either is ever introduced, it must move to `--worktree` scope in the same change.

import { chmodSync, existsSync } from "node:fs";
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

/**
 * The exact files armed executable inside `hooksDir` — never the directory's full listing, so a
 * future README or fixture dropped alongside the hooks is not silently marked executable.
 */
export const HOOK_FILES = ["pre-commit", "pre-push"];

/**
 * Attempts to arm this worktree's hooks and never throws: every failure collapses to
 * `{ armed: false, reason }` instead of propagating, because the caller (the direct-entry block
 * below, wired to `prepare`) must let `pnpm install` succeed regardless of whether hooks could be
 * armed. Hooks are a convenience for a full dev checkout, not a dependency of installing one — a
 * slim container without git, a source tarball, a sparse checkout that omits
 * `scripts/git-hooks/`, or a filesystem where `chmod` fails must all still finish `pnpm install`.
 *
 * The `execFile`/`exists`/`chmod` params exist so a test can drive every failure path (a throwing
 * git invocation, a missing directory, an unwritable file) without spawning a real subprocess or
 * touching the real filesystem.
 */
export function installHooks({
  execFile = execFileSync,
  exists = existsSync,
  chmod = chmodSync,
} = {}) {
  try {
    const root = execFile("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8" }).trim();
    const dir = hooksDir(root);
    if (!exists(dir)) {
      return { armed: false, reason: `${dir} is missing` };
    }
    for (const step of configPlan(root)) {
      execFile("git", step.args, { stdio: "inherit" });
    }
    // core.filemode is false on Windows checkouts, so the bit is set explicitly rather than
    // relying on the checkout to carry it.
    for (const name of HOOK_FILES) {
      const p = join(dir, name);
      if (exists(p)) chmod(p, 0o755);
    }
    return { armed: true, dir };
  } catch (err) {
    return { armed: false, reason: err.message };
  }
}

if (isDirectEntry(import.meta.url)) {
  if (shouldSkip(process.env)) {
    console.log("install-git-hooks: skipped under CI");
    process.exit(0);
  }
  const result = installHooks();
  if (!result.armed) {
    // Exits 0 on every failure path: this runs under `prepare`, and a build-only consumer
    // (no VCS metadata, a sparse checkout, an unwritable filesystem) must still finish
    // installing dependencies. The success message below stays loud so an operator can tell
    // armed from not-armed; this one is deliberately non-fatal but still visible.
    console.warn(`install-git-hooks: hooks not armed (${result.reason})`);
    process.exit(0);
  }
  console.log(`install-git-hooks: core.hooksPath -> ${result.dir}`);
}
