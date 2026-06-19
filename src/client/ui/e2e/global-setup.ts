import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

// Build the artifacts the binary needs before Playwright's webServer runs it: the
// client bundle (embedded/served by the binary) and the `shadowcat` binary itself.
// The webServer (playwright.config.ts) then runs the prebuilt binary and owns its
// lifecycle.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..");

export default async function globalSetup(): Promise<void> {
  run("pnpm", ["--filter", "@shadowcat/ui", "build"]);
  run("cargo", ["build", "-p", "shadowcat", "--bin", "shadowcat"]);
}

function run(cmd: string, args: string[]): void {
  const r = spawnSync(cmd, args, {
    cwd: repoRoot,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (${r.status})`);
}
