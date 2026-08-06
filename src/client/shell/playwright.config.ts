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
  // Budgets sized for the contended 6-worker full-suite run (isolated boots take
  // 3-8s; contention has been measured at 15-53s per test). Without an explicit
  // test timeout, Playwright's 30s default equals the specs' own 30s
  // render-ready assertion budget, so that assertion could never actually use
  // its stated window — the root cause of the render-ready flake class
  // (POST_WORK_FINDINGS "ui-e2e panels reload test flaked once locally").
  timeout: 120_000,
  expect: { timeout: 15_000 },
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
  // `trace` is the only forensic channel for a CI-only failure: the suite runs
  // against a real binary on a runner nobody can attach to, so without a
  // retained trace a red `ui-e2e` is diagnosable only by hypothesis. Scoped to
  // failures so passing runs write nothing. The CI job uploads `test-results/`
  // on failure — dropping that upload step silently re-blinds this setting.
  use: { baseURL: "http://127.0.0.1:31999", trace: "retain-on-failure" },
});
