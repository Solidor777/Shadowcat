import { defineConfig } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

// The built `shadowcat` binary serves the embedded SPA + /api on one origin — the
// faithful e2e target. global-setup builds dist/ + the binary; webServer runs it
// (Playwright owns its lifecycle) with an admin seeded and the setup window off.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const bin = path.join(
  repoRoot,
  "target",
  "debug",
  process.platform === "win32" ? "shadowcat.exe" : "shadowcat",
);

export default defineConfig({
  testDir: "./e2e",
  globalSetup: "./e2e/global-setup.ts",
  webServer: {
    command: `"${bin}"`,
    cwd: repoRoot,
    url: "http://127.0.0.1:31999/health",
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
    env: {
      SHADOWCAT_BIND: "127.0.0.1:31999",
      SHADOWCAT_ADMIN_USER: "ops",
      SHADOWCAT_ADMIN_PASSWORD: "pw-boot",
      SHADOWCAT_SETUP_TOKEN: "off",
      SHADOWCAT_DB: "sqlite::memory:",
      SHADOWCAT_LOG: "warn",
    },
  },
  use: { baseURL: "http://127.0.0.1:31999" },
});
