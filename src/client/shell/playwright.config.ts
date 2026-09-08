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
  // CPU budget. Each worker drives a Chromium rendering a WebGL stage, which costs
  // far more than one core, so Playwright's default (half the logical CPUs) heavily
  // oversubscribes a developer machine and inflates per-test latency roughly 19x:
  // `panels-floating.spec.ts`'s popped-out-arrangement test measures 3.1s at four
  // workers and 59.4s at twelve, on the same machine against a freshly booted server.
  // Oversubscription costs on every axis at once: the full suite measures 146s at four
  // workers with all 34 passing, against 235s at twelve with one failure. The extra
  // workers buy no throughput, make the machine unusable, and push ordinary assertions
  // past `expect`'s budget until they fail on the clock. Four is the measured knee.
  //
  // The hosted runner gets ONE. It has two cores, and a dual-session spec opens two canvases on
  // its own, so any second worker guarantees more canvases than cores — and a starved page stops
  // answering the driver entirely rather than merely rendering late, which surfaces as a spec
  // burning its whole budget instead of failing an assertion.
  //
  // INVARIANT: this cap is what keeps the timeouts below honest. Raising it
  // re-inflates per-test latency and the budgets stop bounding product behaviour.
  workers: process.env.CI === undefined ? 4 : 1,
  // Sized on the WORST observed passing test, not the best: run-to-run spread at the
  // capped worker count is wide (a test measured at 27.1s in one run and 59.3s in
  // another), so a budget fitted to a favourable run leaves no headroom and converts a
  // slow-but-correct run into failures. `expect`'s budget sits well above the slowest
  // single action (0.91s across a 237-action trace) and well under the test budget, so
  // an assertion fails on the assertion rather than on the clock.
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
      // the e2e login pattern itself can never trip them. The `http::throttle` module's
      // production default consts are untouched; this only overrides this webServer process.
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
