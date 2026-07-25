import { defineConfig } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

// The built `shadowcat` binary serves the embedded SPA + /api on one origin — the
// faithful e2e target. The `e2e` npm script builds dist/ + the binary before
// Playwright starts (deterministic; Playwright launches the webServer before any
// globalSetup, so the build must precede `playwright test`). webServer runs the
// prebuilt binary with an admin seeded and the setup window off.
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const bin = path.join(
  repoRoot,
  "target",
  "debug",
  process.platform === "win32" ? "shadowcat.exe" : "shadowcat",
);

export default defineConfig({
  testDir: "./e2e",
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
      // The suite logs in as the same seeded admin identity many times across
      // specs within the auth-throttle's 60s sliding window (Phase A added
      // LOGIN_PER_MIN_PER_IDENTITY=10 to /api/login) — relax the budgets so
      // the e2e login pattern itself can never trip them. Production defaults
      // (config.rs) are untouched; this only overrides this webServer process.
      SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY: "10000",
      SHADOWCAT_LOGIN_PER_MIN_PER_IP: "10000",
      SHADOWCAT_INVITE_PER_MIN_PER_ACCOUNT: "10000",
      SHADOWCAT_INVITE_PER_MIN_PER_IP: "10000",
    },
  },
  use: { baseURL: "http://127.0.0.1:31999" },
});
