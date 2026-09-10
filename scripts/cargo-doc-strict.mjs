// Runs `cargo doc` with a rustdoc lint promoted to an error, in one of two modes, so CI and a
// developer machine execute the same invocation with the same environment.
//
//   warnings — `RUSTDOCFLAGS="-D warnings"` on the stable toolchain; the `docs:api:rust` stage of
//              `pnpm build:all`.
//   examples — `RUSTDOCFLAGS="-D rustdoc::missing_doc_code_examples"` on nightly, into its own
//              target dir; `pnpm docs:check-rust-examples`. The lint is nightly-only, which bears on
//              which toolchain runs it, not on whether a missing example is acceptable — so it
//              denies rather than warns. A separate target dir keeps nightly's artifacts from
//              invalidating stable's fingerprints in `target/`.
//
// The flag lives in this script's own spawned child environment rather than in a workflow `env:`
// block or a `VAR=val cmd` prefix: the gate manifest stores a step's command string alone and the
// local tier runner applies no per-step environment, so any environment carried outside the
// command is environment the local gate silently drops — a lint denied only in CI passes locally
// and fails in CI. `VAR=val cmd` is also not portable to a Windows shell.
// Cross-platform: node:path/node:child_process only; inherits the parent's PATH/env plus the
// added flag, so `cargo` resolves exactly as it would from an unmodified shell invocation.
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";
import { isDirectEntry } from "./lib/is-main.mjs";

export const MODES = {
  warnings: { toolchain: null, rustdocflags: "-D warnings", targetDir: null },
  examples: {
    toolchain: "nightly",
    rustdocflags: "-D rustdoc::missing_doc_code_examples",
    targetDir: ["target", "nightly-doc"],
  },
};

/** The exact `cargo` argv and the environment additions one mode runs — pure, so both modes are directly testable. */
export function cargoDocInvocation(mode, repo) {
  const m = MODES[mode];
  if (!m) {
    throw new Error(`unknown mode: ${mode || "(none)"}. Expected one of ${Object.keys(MODES).join(", ")}`);
  }
  const manifestPath = resolve(repo, "src", "server", "Cargo.toml");
  const args = [
    ...(m.toolchain ? [`+${m.toolchain}`] : []),
    "doc",
    "--manifest-path",
    manifestPath,
    "--document-private-items",
    "--no-deps",
    ...(m.targetDir ? ["--target-dir", resolve(repo, ...m.targetDir)] : []),
  ];
  return { args, env: { RUSTDOCFLAGS: m.rustdocflags } };
}

if (isDirectEntry(import.meta.url)) {
  const repo = resolve(fileURLToPath(import.meta.url), "..", "..");
  const mode = process.argv[2] ?? "warnings";
  let invocation;
  try {
    invocation = cargoDocInvocation(mode, repo);
  } catch (err) {
    console.error(`cargo-doc-strict: ${err.message}`);
    process.exit(2);
  }
  const result = spawnSync("cargo", invocation.args, {
    stdio: "inherit",
    env: { ...process.env, ...invocation.env },
    // Windows resolves `cargo` (no `.exe` suffix given) only through a shell lookup of PATHEXT;
    // POSIX shells resolve it identically either way, so this is on for every platform rather
    // than gated per-OS.
    shell: process.platform === "win32",
  });
  process.exit(result.status ?? 1);
}
