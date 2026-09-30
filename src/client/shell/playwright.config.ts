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

/** The test file that runs only under the `lan-plain-http` project. */
const LAN_SPEC = /lan-plain-http\.spec\.ts$/;

/** A hostname that is not loopback, so a browser treats an `http://` page on it as NOT a secure
 * context, exactly as it does for a LAN address. `--host-resolver-rules` maps it to the loopback
 * test server, so the origin is genuinely insecure with no network setup on any CI OS. */
const LAN_HOST = "shadowcat.test";

/** Port of the non-loopback test server the `lan-plain-http` project targets. */
const LAN_PORT = 32000;

/** Chromium launch flags shared by every project. `--use-gl=angle` selects the platform's real GL
 * backend instead of the CPU rasterizer, and ANGLE falls back to SwiftShader by itself where no
 * device exists, so the flag is safe on a GPU-less runner rather than something to gate per
 * environment. `--autoplay-policy=no-user-gesture-required`: the audio engine's `AudioContext`
 * is constructed before any click, and Chromium's default gesture requirement would otherwise
 * keep it suspended for the rest of the test no matter what gets clicked afterward. */
const BASE_LAUNCH_ARGS = ["--use-gl=angle", "--autoplay-policy=no-user-gesture-required"];

/** Environment shared by every test server process; each server adds its own `SHADOWCAT_BIND`. */
const SERVER_ENV = {
  SHADOWCAT_ADMIN_USER: "ops",
  SHADOWCAT_ADMIN_PASSWORD: "pw-boot",
  SHADOWCAT_SETUP_TOKEN: "off",
  SHADOWCAT_DB: "sqlite::memory:",
  SHADOWCAT_LOG: "warn",
  // The suite logs in as the same seeded admin identity many times across
  // specs within the auth-throttle's 60s sliding window (`/api/login` allows
  // LOGIN_PER_MIN_PER_IDENTITY=10 by default) — relax the budgets so
  // the e2e login pattern itself can never trip them. The `http::throttle` module's
  // production default consts are untouched; this only overrides this webServer process.
  SHADOWCAT_LOGIN_PER_MIN_PER_IDENTITY: "10000",
  SHADOWCAT_LOGIN_PER_MIN_PER_IP: "10000",
  SHADOWCAT_INVITE_PER_MIN_PER_ACCOUNT: "10000",
  SHADOWCAT_INVITE_PER_MIN_PER_IP: "10000",
  // Same reasoning as the login/invite overrides above, for `POST /api/worlds`'s
  // per-account budget (`WORLD_CREATE_PER_MIN_PER_ACCOUNT=10`): each worker's
  // seeded identity creates far more than 10 worlds across the whole suite, and
  // `reuseExistingServer` keeps one server (and its sliding-window throttle state)
  // alive across repeated local invocations of a subset of specs.
  SHADOWCAT_WORLD_CREATE_PER_MIN_PER_ACCOUNT: "10000",
};

export default defineConfig({
  testDir: "./e2e",
  projects: [
    { name: "loopback", testIgnore: LAN_SPEC },
    {
      // Plain-HTTP LAN play: the served origin is not a secure context, so every
      // `[SecureContext]` browser API is absent and the browser itself decides which.
      name: "lan-plain-http",
      testMatch: LAN_SPEC,
      use: {
        baseURL: `http://${LAN_HOST}:${LAN_PORT}`,
        launchOptions: { args: [...BASE_LAUNCH_ARGS, `--host-resolver-rules=MAP ${LAN_HOST} 127.0.0.1`] },
      },
    },
  ],
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
  webServer: [
    {
      command: `"${bin}"`,
      cwd: repoRoot,
      url: "http://127.0.0.1:31999/health",
      timeout: 120_000,
      reuseExistingServer: !process.env.CI,
      env: { ...SERVER_ENV, SHADOWCAT_BIND: "127.0.0.1:31999" },
    },
    {
      // The documented LAN configuration: a bind beyond loopback. Server policy keyed on a
      // non-loopback bind (`Config::is_exposed`) applies only here, so the `lan-plain-http`
      // project runs against this bind, not the loopback one.
      command: `"${bin}"`,
      cwd: repoRoot,
      url: `http://127.0.0.1:${LAN_PORT}/health`,
      timeout: 120_000,
      reuseExistingServer: !process.env.CI,
      env: { ...SERVER_ENV, SHADOWCAT_BIND: `0.0.0.0:${LAN_PORT}` },
    },
  ],
  // `trace` is the only forensic channel for a CI-only failure: the suite runs
  // against a real binary on a runner nobody can attach to, so without a
  // retained trace a red `ui-e2e` is diagnosable only by hypothesis. Scoped to
  // failures so passing runs write nothing. The CI job uploads `test-results/`
  // on failure — dropping that upload step silently re-blinds this setting.
  use: {
    baseURL: "http://127.0.0.1:31999",
    trace: "retain-on-failure",
    // Chromium launch flags (GL backend, autoplay policy) are documented on `BASE_LAUNCH_ARGS`.
    launchOptions: { args: BASE_LAUNCH_ARGS },
  },
});
