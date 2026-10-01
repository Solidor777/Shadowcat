import { defineConfig } from "@playwright/test";
import type { PlaywrightTestConfig } from "@playwright/test";
import path from "node:path";
import { networkInterfaces } from "node:os";
import { fileURLToPath } from "node:url";
import { pickLanAddress } from "./e2e-support/lan-address";
import {
  ANY_HTTPS_SPEC,
  HTTPS_SPECS,
  LAN_SPEC,
  UPGRADE_CHROMIUM_SPEC,
  UPGRADE_SPEC,
} from "./e2e-support/spec-matchers";

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

/** A hostname that is not loopback, so a browser treats an `http://` page on it as NOT a secure
 * context, exactly as it does for a LAN address. `--host-resolver-rules` maps it to the loopback
 * test server, so the origin is genuinely insecure with no network setup on any CI OS. */
const LAN_HOST = "shadowcat.test";

/** Port of the non-loopback test server the `lan-plain-http` project targets. */
const LAN_PORT = 32000;

/** Port of the test server the built-in-HTTPS projects target: bound beyond loopback with TLS on
 * its default (`auto`), so one port answers both `https://` and plain `http://`. */
const HTTPS_PORT = 32001;

/** The machine's real non-loopback IPv4 address. The HTTPS specs browse to it rather than to
 * loopback or a resolver-rule alias, because browsers treat loopback as potentially trustworthy
 * and would hide the plain-HTTP behaviour (no secure context, cookie scoping) these specs assert;
 * a `--host-resolver-rules` alias is Chromium-only and cannot serve Firefox or WebKit. */
const LAN_IP = resolveLanIp();

/** One Playwright project definition. */
type Project = NonNullable<PlaywrightTestConfig["projects"]>[number];

/**
 * Resolves the LAN address without letting a machine that has none take down the whole suite.
 * On CI a missing address is a hard failure (the HTTPS projects are part of the gate); elsewhere
 * the LAN/HTTPS projects and their server are omitted with a loud message, so the loopback suite
 * still runs on a loopback-only host.
 * @returns The address, or `undefined` when none exists and the run is not on CI.
 * @example
 * ```
 * const ip = resolveLanIp();
 * ```
 */
function resolveLanIp(): string | undefined {
  try {
    return pickLanAddress(networkInterfaces(), process.env.SHADOWCAT_E2E_LAN_IP);
  } catch (e) {
    if (process.env.CI !== undefined) throw e;
    console.warn(
      `
!!! HTTPS e2e projects OMITTED: ${(e as Error).message}. Only the loopback and lan-plain-http projects run. !!!
`,
    );
    return undefined;
  }
}

/** A DNS name (RFC 6761 `.example`, never registered) the upgrade projects map to this machine so
 * an engine sees a real hostname rather than an address literal. */
const UPGRADE_HOST = "upgrade.shadowcat.example";

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

/** The built-in-HTTPS and upgrade projects, which browse to the machine's real interface address.
 * @param LAN_IP The address the projects target.
 * @returns The project definitions.
 * @example
 * ```
 * const projects = httpsProjects("192.0.2.1");
 * ```
 */
function httpsProjects(LAN_IP: string): Project[] {
  return [
    // Built-in HTTPS on the machine's real interface address, once per engine. The self-signed
    // certificate is untrusted by design, so `ignoreHTTPSErrors` is on except in the specs whose
    // subject is trust itself (they `test.use` it off).
    {
      name: "https-chromium",
      testMatch: HTTPS_SPECS,
      use: {
        browserName: "chromium",
        baseURL: `https://${LAN_IP}:${HTTPS_PORT}`,
        ignoreHTTPSErrors: true,
        launchOptions: { args: BASE_LAUNCH_ARGS },
      },
    },
    {
      name: "https-firefox",
      testMatch: HTTPS_SPECS,
      use: {
        browserName: "firefox",
        baseURL: `https://${LAN_IP}:${HTTPS_PORT}`,
        ignoreHTTPSErrors: true,
      },
    },
    {
      name: "https-webkit",
      testMatch: HTTPS_SPECS,
      use: {
        browserName: "webkit",
        baseURL: `https://${LAN_IP}:${HTTPS_PORT}`,
        ignoreHTTPSErrors: true,
      },
    },
    // Typing the plain `http://` address into a browser that upgrades to HTTPS: each engine runs
    // with its upgrade mode ON (Chromium's HTTPS-First Mode through its own profile, Firefox's
    // HTTPS-Only Mode; WebKit has no switch and runs its default). Address shapes:
    //   - the machine's bare IP (what a phone types), where engines exempt private addresses;
    //   - a DNS name the engine does upgrade, resolved to this machine (Chromium via
    //     `--host-resolver-rules`, Firefox via `network.dns.localDomains`; WebKit has no resolver
    //     override, so it has no named project);
    //   - Firefox's non-default `upgrade_local`, which upgrades private addresses too.
    {
      name: "https-upgrade-chromium",
      testMatch: UPGRADE_CHROMIUM_SPEC,
      use: { browserName: "chromium", baseURL: `http://${LAN_IP}:${HTTPS_PORT}` },
    },
    {
      name: "https-upgrade-chromium-named",
      testMatch: UPGRADE_CHROMIUM_SPEC,
      use: {
        browserName: "chromium",
        baseURL: `http://${UPGRADE_HOST}:${HTTPS_PORT}`,
        launchOptions: { args: [`--host-resolver-rules=MAP ${UPGRADE_HOST} ${LAN_IP}`] },
      },
    },
    {
      name: "https-upgrade-firefox",
      testMatch: UPGRADE_SPEC,
      use: {
        browserName: "firefox",
        baseURL: `http://${LAN_IP}:${HTTPS_PORT}`,
        launchOptions: { firefoxUserPrefs: { "dom.security.https_only_mode": true } },
      },
    },
    {
      name: "https-upgrade-firefox-named",
      testMatch: UPGRADE_SPEC,
      use: {
        browserName: "firefox",
        baseURL: `http://${UPGRADE_HOST}:${HTTPS_PORT}`,
        launchOptions: {
          firefoxUserPrefs: {
            "dom.security.https_only_mode": true,
            "network.dns.localDomains": UPGRADE_HOST,
          },
        },
      },
    },
    {
      name: "https-upgrade-firefox-local",
      testMatch: UPGRADE_SPEC,
      use: {
        browserName: "firefox",
        baseURL: `http://${LAN_IP}:${HTTPS_PORT}`,
        launchOptions: {
          firefoxUserPrefs: {
            "dom.security.https_only_mode": true,
            "dom.security.https_only_mode.upgrade_local": true,
          },
        },
      },
    },
    {
      name: "https-upgrade-webkit",
      testMatch: UPGRADE_SPEC,
      use: { browserName: "webkit", baseURL: `http://${LAN_IP}:${HTTPS_PORT}` },
    },
  ];
}

/** The test server the HTTPS projects target.
 * @param ip The address the readiness probe dials.
 * @returns The webServer definition.
 * @example
 * ```
 * const server = httpsServer("192.0.2.1");
 * ```
 */
function httpsServer(ip: string) {
  return (
    {
      // Built-in HTTPS: TLS at its default on a non-loopback bind, so this one port serves
      // `https://` and plain `http://` from the machine's real interface address. The readiness
      // probe is plain HTTP, which the shared port also answers.
      command: `"${bin}"`,
      cwd: repoRoot,
      url: `http://${ip}:${HTTPS_PORT}/health`,
      timeout: 120_000,
      reuseExistingServer: !process.env.CI,
      env: { ...SERVER_ENV, SHADOWCAT_BIND: `0.0.0.0:${HTTPS_PORT}` },
    }
  );
}

export default defineConfig({
  testDir: "./e2e",
  projects: [
    { name: "loopback", testIgnore: [LAN_SPEC, ...ANY_HTTPS_SPEC] },
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
    ...(LAN_IP === undefined ? [] : httpsProjects(LAN_IP)),
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
    ...(LAN_IP === undefined ? [] : [httpsServer(LAN_IP)]),
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
