// Runs a tier of the gate manifest, and records a receipt the pre-push hook verifies.
//
// The push tier costs ~19 minutes, which cannot run inside `git push`: an agent's shell tool
// caps at 600 seconds, and a killed push is exactly the pressure that produces `--no-verify`.
// So the tier runs explicitly and leaves a receipt; the hook checks the receipt.
//
// The receipt is keyed to a TREE HASH, never a timestamp. Gates run against a working tree, so
// a receipt keyed to time can describe a tree other than the one being pushed — the defect this
// mechanism exists to remove.
//
// Order is workflow order, so the client build precedes the cargo steps that embed dist/:
// `tierCommands` never reorders `entries`, it only filters and dedupes, so INCLUDED's ordering
// guarantee reduces to `parseGateManifest`'s own emission order — the manifest already lists
// `pnpm build` before every cargo step in the push tier.

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

export function readReceipt(gitDir) {
  const p = receiptPath(gitDir);
  if (!existsSync(p)) return null;
  try {
    return JSON.parse(readFileSync(p, "utf8"));
  } catch {
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

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

if (isDirectEntry(import.meta.url)) {
  const [mode, arg] = process.argv.slice(2);
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

  const commands = tierCommands(entries, mode);
  console.log(`gate:${mode} — ${commands.length} step(s)`);
  for (const cmd of commands) {
    const started = Date.now();
    process.stdout.write(`  ${cmd} ... `);
    try {
      execFileSync(cmd, { shell: true, stdio: ["ignore", "pipe", "pipe"] });
    } catch (err) {
      console.log("FAIL");
      process.stderr.write(String(err.stdout ?? "") + String(err.stderr ?? ""));
      console.error(`\ngate:${mode} FAILED at: ${cmd}`);
      process.exit(1);
    }
    console.log(`ok ${Math.round((Date.now() - started) / 1000)}s`);
  }

  if (mode === "push") {
    writeReceipt(gitDir, {
      tree: git("rev-parse", "HEAD^{tree}"),
      sha: git("rev-parse", "HEAD"),
      manifest: hash,
      finishedAt: new Date().toISOString(),
    });
    console.log("gate:push green — receipt written");
  }
}
