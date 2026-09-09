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
// Order is workflow order, so the client build precedes the cargo steps that embed dist/:
// `tierCommands` never reorders `entries`, it only filters and dedupes, so INCLUDED's ordering
// guarantee reduces to `parseGateManifest`'s own emission order — the manifest already lists
// `pnpm build` before every cargo step in the push tier (pinned by a test against the real
// manifest, not just synthetic fixtures, so a future reordering in `gates.toml` fails the suite).

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { join } from "node:path";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";
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

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

if (isDirectEntry(import.meta.url)) {
  const [mode, arg] = process.argv.slice(2);

  if (mode === "--verify-receipt" && !arg) {
    console.error("usage: node scripts/run-gate-tier.mjs --verify-receipt <tree>");
    process.exit(2);
  }

  const text = readFileSync(MANIFEST, "utf8");
  const entries = parseGateManifest(text, MANIFEST);
  const hash = manifestHash(text);
  const gitDir = git("rev-parse", "--absolute-git-dir");

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
  if (mode === "push") {
    const status = git("status", "--porcelain");
    const refusal = pushDirtyTreeRefusal(status);
    if (!refusal.ok) {
      console.error(refusal.why);
      process.exit(1);
    }
    startHead = git("rev-parse", "HEAD");
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
    const endStatusRefusal = pushDirtyTreeRefusalAfterRun(git("status", "--porcelain"));
    if (!endStatusRefusal.ok) {
      console.error(endStatusRefusal.why);
      process.exit(1);
    }
    const endHead = git("rev-parse", "HEAD");
    const headRefusal = headMovedRefusal(startHead, endHead);
    if (!headRefusal.ok) {
      console.error(headRefusal.why);
      process.exit(1);
    }
    writeReceipt(gitDir, {
      tree: git("rev-parse", "HEAD^{tree}"),
      sha: endHead,
      manifest: hash,
      finishedAt: new Date().toISOString(),
    });
    console.log("gate:push green — receipt written");
  }
}
