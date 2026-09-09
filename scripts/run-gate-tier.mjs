// Runs a tier of the gate manifest, and records a receipt the pre-push hook verifies.
//
// The push tier costs ~19 minutes, which cannot run inside `git push`: an agent's shell tool
// caps at 600 seconds, and a killed push is exactly the pressure that produces `--no-verify`.
// So the tier runs explicitly and leaves a receipt; the hook checks the receipt.
//
// The receipt is keyed to a TREE HASH, never a timestamp. Gates run against a working tree, so
// a receipt keyed to time can describe a tree other than the one being pushed — the defect this
// mechanism exists to remove. But a tree hash alone is not enough: `push` mode RUNS the gates
// against the working tree and then STAMPS the receipt with `HEAD^{tree}`. If the tree is dirty
// at run time those two are different trees — an uncommitted fix can turn a red HEAD green for
// the run, get discarded afterward, and leave a receipt that verifies a clean tree nobody tested.
// So `push` mode refuses outright on a dirty tree, checked BEFORE any gate command executes;
// checking after the fact cannot retroactively prove what was actually tested.
//
// A pre-run check alone still leaves a ~19-minute TOCTOU window open: a commit landing, or a
// tree edit, between the pre-run check and `writeReceipt` produces the exact same defect — the
// receipt names a tree the gates never ran against as a whole. Two other windows in this repo's
// own history record edits landing under a backgrounded gate chain, so this is not hypothetical.
// The fix is symmetric: capture `git status --porcelain` and `git rev-parse HEAD` BEFORE the
// run, and re-check both immediately before `writeReceipt` — refusing if either moved.
//
// A per-field check is not enough either: `sha` and `tree` were each read by their OWN
// `git rev-parse` call, so even with the dirty/HEAD-moved checks passing, the two values could
// still be sampled from two different instants a process switch apart — the receipt could name
// a `sha` and a `tree` that never coexisted. The general lesson, not just this one instance: a
// receipt field sampled by its own independent `git` invocation is never provably simultaneous
// with any other field's sample. So EVERY receipt field is derived from ONE atomic capture —
// a single `git rev-parse HEAD HEAD^{tree}` call, one process, one ref read — never from a
// second, later, or field-specific `git` call. See `buildReceipt`'s own comment for the
// invariant this enforces at the one place a receipt is assembled.
//
// Sampling `sha`/`tree`/status/HEAD at two points is still not the general answer: any tracked
// file that is edited and REVERTED entirely between the pre-run and post-run samples is invisible
// to every point-in-time check above — HEAD never moves, `git status --porcelain` reads clean at
// both samples, and the receipt certifies a tree some gate command never actually ran against.
// This repo's own history records that exact operational shape twice. No amount of additional
// point sampling closes this; the fix is a continuous-coverage check instead: capture every
// tracked file's `(path, size, mtime)` before the run and again before `writeReceipt`, and refuse
// if anything changed — a revert still moves the file's mtime even though its final content
// matches. One family of tracked files needs a DERIVED exemption: the push tier itself
// regenerates `src/types/generated` (`cargo test --all` runs ts-rs) while leaving `git status`
// clean, so a naive table refuses every real push. The exemption is read out of the manifest's
// own `git diff --exit-code <path>` entry (the bindings-sync gate already asserts that path stays
// generated-and-clean) rather than hardcoded, so it tracks that entry automatically and an absent
// entry yields an EMPTY exemption set — failing toward stricter, never toward permissive.
//
// Order is workflow order, so the client build precedes the cargo steps that embed dist/:
// `tierCommands` never reorders `entries`, it only filters and dedupes, so INCLUDED's ordering
// guarantee reduces to `parseGateManifest`'s own emission order — the manifest already lists
// `pnpm build` before every cargo step in the push tier (pinned by a test against the real
// manifest, not just synthetic fixtures, so a future reordering in `gates.toml` fails the suite).

import { readFileSync, writeFileSync, existsSync, lstatSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
import { runGit } from "./lib/run-git.mjs";
import { parseGateManifest, MANIFEST } from "./check-gate-manifest.mjs";

const INCLUDED = { commit: ["commit"], push: ["commit", "push"] };

/** The commands a tier runs, deduped to first occurrence, in workflow order. */
export function tierCommands(entries, tier) {
  const want = INCLUDED[tier];
  if (!want) throw new Error(`unknown tier: ${tier}`);
  const seen = new Set();
  const out = [];
  for (const e of entries) {
    if (!want.includes(e.tier) || seen.has(e.command)) continue;
    seen.add(e.command);
    out.push(e.command);
  }
  return out;
}

/** Identity of the gate list; a changed manifest invalidates every outstanding receipt. */
export function manifestHash(text) {
  return createHash("sha256").update(text).digest("hex").slice(0, 16);
}

export function receiptPath(gitDir) {
  return join(gitDir, "shadowcat-gate-receipt");
}

export function writeReceipt(gitDir, receipt) {
  writeFileSync(receiptPath(gitDir), JSON.stringify(receipt, null, 2) + "\n");
}

/**
 * Reads the receipt, or `null` when none exists or the file cannot be parsed. Both cases collapse
 * to `null` for the caller's control flow (missing and corrupt both mean "no receipt to trust"),
 * but a corrupt file is a different problem for the OPERATOR than a merely absent one — recreating
 * it costs the full push-tier run, so the distinction is worth a diagnostic even though the return
 * value can't carry it without breaking every existing caller's `object | null` contract.
 */
export function readReceipt(gitDir) {
  const p = receiptPath(gitDir);
  if (!existsSync(p)) return null;
  try {
    return JSON.parse(readFileSync(p, "utf8"));
  } catch (err) {
    console.error(`gate: receipt at ${p} is corrupt (${err.message}) — treating as missing: re-run \`pnpm gate:push\``);
    return null;
  }
}

/** Whether a receipt describes exactly this tree under exactly this gate list. */
export function receiptMatches(receipt, { tree, manifest }) {
  if (!receipt) return { ok: false, why: "no receipt: run `pnpm gate:push`" };
  if (receipt.tree !== tree) {
    return {
      ok: false,
      why: `receipt is for tree ${receipt.tree}, not ${tree}: re-run \`pnpm gate:push\``,
    };
  }
  if (receipt.manifest !== manifest) {
    return { ok: false, why: "the gate manifest changed since the receipt: re-run `pnpm gate:push`" };
  }
  return { ok: true, why: "" };
}

/**
 * Whether `push` mode may proceed: a dirty tree means whatever the gates run against cannot be
 * proven identical to the tree the receipt will name (`HEAD^{tree}`), so refusal is the only
 * sound answer — checked against `git status --porcelain` BEFORE the first gate command runs.
 */
export function pushDirtyTreeRefusal(porcelainStatus) {
  const dirty = porcelainStatus.trim() !== "";
  return {
    ok: !dirty,
    why: dirty
      ? "gate: refusing to run the push tier on a dirty tree — uncommitted changes mean a green run cannot be proven to describe HEAD's tree. Commit or stash, then re-run `pnpm gate:push`."
      : "",
  };
}

/**
 * Whether the tree became dirty DURING a push-tier run — the same check as
 * `pushDirtyTreeRefusal`, re-run immediately before `writeReceipt` rather than only before the
 * first gate command, since a pre-run-only check leaves the whole run's duration racy.
 */
export function pushDirtyTreeRefusalAfterRun(porcelainStatus) {
  const dirty = porcelainStatus.trim() !== "";
  return {
    ok: !dirty,
    why: dirty
      ? "gate: refusing to write the receipt — the working tree became dirty during the run, so the gates cannot be proven to describe what's being pushed. Commit or stash, then repeat `pnpm gate:push`."
      : "",
  };
}

/** Whether HEAD moved between the start and end of a push-tier run — a race the receipt must refuse. */
export function headMovedRefusal(startHead, endHead) {
  const moved = startHead !== endHead;
  return {
    ok: !moved,
    why: moved
      ? `gate: refusing to write the receipt — HEAD moved during the run (was ${startHead}, now ${endHead}), so the gates did not run against the commit now at HEAD. Repeat \`pnpm gate:push\`.`
      : "",
  };
}

/**
 * Reads the tracked-file mtime-table exemption out of the manifest's own `git diff --exit-code
 * <path>` entry (the bindings-sync gate), rather than hardcoding a path. An entry that is ever
 * removed or repointed carries the exemption with it automatically; an ABSENT entry yields an
 * EMPTY exemption array, so a manifest edit that drops the entry fails the mtime-table check
 * toward stricter, never toward silently permissive.
 */
export function derivedMtimeExemptions(entries) {
  const out = [];
  for (const e of entries) {
    const m = e.command.match(/^git diff --exit-code (\S+)$/);
    if (m) out.push(m[1]);
  }
  return out;
}

/**
 * Whether any tracked file — outside the derived exemptions — changed size or mtime between two
 * captured `{ path: { size, mtimeNs } | null }` tables. Catches an edit that lands and is fully
 * REVERTED within the run: content matches at both samples, so `git status` never sees it, but a
 * revert still moves the file's mtime, which this compares directly rather than inferring from
 * git. `mtimeNs` is a bigint compared exactly, so two stamps differ here whenever the OS reports
 * them differently. A path present in only one table (created or deleted between samples) counts
 * as changed, and so does a `null` entry (the path could not be stat'd) in EITHER table — an
 * unreadable path fails toward refusal, never toward sitting silently outside the check.
 */
export function fileTableChangedRefusal(before, after, exemptPrefixes = []) {
  const isExempt = (p) =>
    exemptPrefixes.some((prefix) => p === prefix || p.startsWith(`${prefix.replace(/\/$/, "")}/`));
  const paths = new Set([...Object.keys(before), ...Object.keys(after)]);
  const changed = [];
  for (const p of paths) {
    if (isExempt(p)) continue;
    const b = before[p];
    const a = after[p];
    if (!b || !a || b.size !== a.size || b.mtimeNs !== a.mtimeNs) changed.push(p);
  }
  changed.sort();
  return {
    ok: changed.length === 0,
    why:
      changed.length > 0
        ? `gate: refusing to write the receipt — ${changed.length} tracked file(s) changed on disk, appeared, disappeared, or could not be read during the run (e.g. ${changed[0]}), even though HEAD and \`git status\` both read clean. This can happen from an edit that landed and was reverted mid-run. Repeat \`pnpm gate:push\`.`
        : "",
  };
}

/**
 * Splits `git rev-parse HEAD HEAD^{tree}`'s two-line stdout into its sha and tree. One
 * invocation resolves both refs against the SAME repository state, so the two lines can never
 * describe two different instants the way two separate `git rev-parse` calls could.
 */
export function parseHeadAndTree(stdout) {
  const [sha, tree] = stdout.trim().split("\n").map((l) => l.trim());
  return { sha, tree };
}

/**
 * Assembles the receipt from ONE captured `{ sha, tree }` sample plus the manifest hash. This is
 * the single place a receipt is built, and that is load-bearing: every field here comes from the
 * SAME captured sample, or from data captured at this same instant (the manifest hash,
 * `finishedAt`), never from its own independent lookup. INVARIANT: a field that calls `git` on
 * its own, here or anywhere else, reintroduces a split-sample receipt — two fields that are each
 * individually valid while together describing an instant that never existed. Route any new
 * field through the captured sample instead of adding a call.
 *
 * This function CANNOT enforce that invariant by itself: given `{ sha, tree }`, it has no way to
 * tell whether both came from one `git rev-parse HEAD HEAD^{tree}` call or were bundled from two
 * independent calls at the call site before being passed in — the object looks identical either
 * way. The real enforcement is the `GIT-CALL-BUDGET-START`/`-END` markers bracketing the call
 * site below, plus the test in `run-gate-tier.test.mjs` that scans between them and fails if more
 * than one `git(` call appears. That is a documented convention checked by a source-scanning
 * test, not a property this function's own unit tests can verify — they can only confirm this
 * function doesn't drop or swap the fields it's handed.
 */
export function buildReceipt({ sha, tree }, manifest, finishedAt = new Date().toISOString()) {
  return { tree, sha, manifest, finishedAt };
}

// Impure capture helpers — mirrors the git-invoking helpers below: real filesystem/`git` reads,
// kept out of the pure comparison functions above so those stay directly testable without
// touching disk.

// NUL-delimited so a path needing quoting (non-ASCII, `core.quotepath`) arrives verbatim: a
// quoted path would fail to stat at both samples and refuse every push.
// `gitCall` is a (...args) => string callback, not the shared `runGit` import directly — the
// caller supplies one bound to its own mode-appropriate abort behavior (see `gitOrAbort` below).
function listTrackedFiles(gitCall) {
  return gitCall("ls-files", "-z").split("\0").filter(Boolean);
}

/**
 * Captures `{ size, mtimeNs }` for every path, keyed by path, from each entry's OWN inode via
 * `lstatSync`. A tracked symlink is a blob whose content is its target path string, so editing
 * and reverting one recreates the LINK and moves the link's mtime while the target's inode never
 * changes; a following `stat` would watch the wrong object. The link's own size and mtime are
 * what is compared — its target string is content, and content equality at both samples is what
 * `git status` already establishes; this table exists for the time dimension content equality
 * cannot see, so reading the target string here would fork that decision.
 *
 * `mtimeNs` is a bigint, exact at whatever resolution the OS reports; the `mtimeMs` double has a
 * ~244 ns spacing at the current epoch and folds distinct nanosecond stamps together. LIMIT: the
 * filesystem's own timestamp clock still bounds detection. Measured on Windows/NTFS, successive
 * writes land on stamps no closer than ~0.3 ms, so an edit fully reverted within one clock step is
 * invisible to this table; Linux and macOS have their own clock granularities, unmeasured here.
 *
 * A path that cannot be stat'd records `null` rather than being left out, so
 * `fileTableChangedRefusal` counts it as changed even when it is unreadable at BOTH samples.
 */
export function captureFileTable(paths) {
  const table = {};
  for (const p of paths) {
    try {
      const st = lstatSync(p, { bigint: true });
      table[p] = { size: st.size, mtimeNs: st.mtimeNs };
    } catch {
      table[p] = null;
    }
  }
  return table;
}

/**
 * Runs a git command for `mode`, aborting legibly (never a raw stack trace) on failure — every
 * git invocation in this direct-entry block routes through this rather than calling `runGit`
 * (`scripts/lib/run-git.mjs`) inline, because the shared helper deliberately reports failure
 * without deciding what it means, and each of this plan's three entry points needs a DIFFERENT
 * safe direction on failure. Here, specifically: `--verify-receipt` is called from `pre-push` to
 * decide whether a push may proceed, so a git failure means it cannot verify anything — refusing
 * the push is the only safe answer, worded the same as every other push refusal so an operator
 * cannot tell this apart from a real gate failure by wording alone. `commit`/`push` modes cannot
 * safely continue without knowing the git state either, so they abort the same way.
 */
function gitOrAbort(args, what, mode) {
  const result = runGit(args, what);
  if (result.ok) return result.stdout;
  const prefix = mode === "--verify-receipt" ? "gate: refusing the push" : `gate:${mode} aborted`;
  console.error(`${prefix} — ${result.message}`);
  process.exit(1);
}

if (isDirectEntry(import.meta.url)) {
  const [mode, arg] = process.argv.slice(2);

  if (mode === "--verify-receipt" && !arg) {
    console.error("usage: node scripts/run-gate-tier.mjs --verify-receipt <tree>");
    process.exit(2);
  }

  const text = readFileSync(MANIFEST, "utf8");
  const entries = parseGateManifest(text, MANIFEST);
  const hash = manifestHash(text);
  const gitDir = gitOrAbort(["rev-parse", "--absolute-git-dir"], "the git directory", mode);

  if (mode === "--verify-receipt") {
    const r = receiptMatches(readReceipt(gitDir), { tree: arg, manifest: hash });
    if (!r.ok) {
      console.error(`gate: refusing the push — ${r.why}`);
      process.exit(1);
    }
    console.log("gate: receipt matches the tree being pushed");
    process.exit(0);
  }

  let startHead = null;
  let trackedFiles = [];
  let startTable = {};
  let mtimeExemptions = [];
  if (mode === "push") {
    const status = gitOrAbort(["status", "--porcelain"], "the working tree status", mode);
    const refusal = pushDirtyTreeRefusal(status);
    if (!refusal.ok) {
      console.error(refusal.why);
      process.exit(1);
    }
    startHead = gitOrAbort(["rev-parse", "HEAD"], "HEAD", mode);
    mtimeExemptions = derivedMtimeExemptions(entries);
    trackedFiles = listTrackedFiles((...args) => gitOrAbort(args, "the tracked-file list", mode));
    startTable = captureFileTable(trackedFiles);
  }

  const commands = tierCommands(entries, mode);
  console.log(`gate:${mode} — ${commands.length} step(s)`);
  for (const cmd of commands) {
    const started = Date.now();
    console.log(`  ${cmd}`);
    try {
      // Streams live rather than buffering: a hung step (this codebase has a recorded history of
      // suites hanging) must show the operator real output, not a frozen prefix, and `stdio:
      // "inherit"` also removes execFileSync's default 10MB maxBuffer ceiling — a real limit
      // against a verbose step like `cargo test --all` or `cargo +nightly doc`.
      execFileSync(cmd, { shell: true, stdio: "inherit" });
    } catch (err) {
      console.error(`\ngate:${mode} FAILED at: ${cmd}\n${err.message}`);
      process.exit(1);
    }
    console.log(`  ok ${Math.round((Date.now() - started) / 1000)}s`);
  }

  if (mode === "push") {
    // Re-checked here rather than trusted from before the run: the run just took up to ~19
    // minutes, and either check moving during that window means the receipt is about to name a
    // tree the gates never actually ran against as a whole.
    const endStatus = gitOrAbort(["status", "--porcelain"], "the working tree status", mode);
    const endStatusRefusal = pushDirtyTreeRefusalAfterRun(endStatus);
    if (!endStatusRefusal.ok) {
      console.error(endStatusRefusal.why);
      process.exit(1);
    }
    // Catches an edit-and-revert within the run: content matches at both point samples above, so
    // status/HEAD stay clean, but a revert still moves the file's mtime. Compared against the
    // SAME `trackedFiles` path list captured before the run, so this is a fixed-path-set diff,
    // not a re-listing that could itself drift.
    const endTable = captureFileTable(trackedFiles);
    const tableRefusal = fileTableChangedRefusal(startTable, endTable, mtimeExemptions);
    if (!tableRefusal.ok) {
      console.error(tableRefusal.why);
      process.exit(1);
    }
    // GIT-CALL-BUDGET-START — exactly one call to `gitOrAbort` may appear before the closing
    // marker below (the atomic sha/tree capture). `buildReceipt`'s own comment explains why this
    // is enforced here by a source-scanning test rather than by that function's unit tests.
    const captured = parseHeadAndTree(
      gitOrAbort(["rev-parse", "HEAD", "HEAD^{tree}"], "the atomic HEAD/tree sample", mode),
    );
    const headRefusal = headMovedRefusal(startHead, captured.sha);
    if (!headRefusal.ok) {
      console.error(headRefusal.why);
      process.exit(1);
    }
    writeReceipt(gitDir, buildReceipt(captured, hash));
    // GIT-CALL-BUDGET-END
    console.log("gate:push green — receipt written");
  }
}
