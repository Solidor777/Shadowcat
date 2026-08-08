// Runs `cargo doc` with rustdoc warnings promoted to errors, matching the CI docs job's
// `RUSTDOCFLAGS: "-D warnings"` env var on its `pnpm build:all` step. `pnpm docs:api:rust` alone
// carries no such flag, so a bare local `cargo doc` (or `pnpm build:all` run locally) went green
// on a rustdoc warning that would redden the pipeline — the exact same local/CI asymmetry this
// docs-hardening work removed everywhere else. `RUSTDOCFLAGS="-D warnings" cargo doc` inline in
// package.json is not portable to a Windows shell (no `VAR=val cmd` syntax there), so the flag is
// set in this script's own spawned child environment instead.
// Cross-platform: node:path/node:child_process only; inherits the parent's PATH/env plus the
// added flag, so `cargo` resolves exactly as it would from an unmodified shell invocation.
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
const manifestPath = resolve(repo, "src", "server", "Cargo.toml");

const result = spawnSync(
  "cargo",
  ["doc", "--manifest-path", manifestPath, "--document-private-items", "--no-deps"],
  {
    stdio: "inherit",
    env: { ...process.env, RUSTDOCFLAGS: "-D warnings" },
    // Windows resolves `cargo` (no `.exe` suffix given) only through a shell lookup of PATHEXT;
    // POSIX shells resolve it identically either way, so this is on for every platform rather
    // than gated per-OS.
    shell: process.platform === "win32",
  },
);

process.exit(result.status ?? 1);
