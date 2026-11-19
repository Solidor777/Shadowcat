import { test, expect, chromium } from "@playwright/test";
import type { BrowserContext } from "@playwright/test";
import { mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { INVOKE_TIMEOUT_MS, invokeBrowserButton } from "../e2e-support/browser-ui";
import { launchWindowedChromium } from "../e2e-support/windowed-chromium";
import { requirePrivateAddress } from "./https-support";

// Chromium's HTTPS-First Mode ("Always use secure connections") upgrades an explicit `http://`
// navigation, including one to a bare LAN address, and shows a browser-owned interstitial when the
// upgraded attempt fails. Playwright's default headless shell does not implement it, and the
// `HttpsUpgrades` launch feature only upgrades scheme-less omnibox input, so neither exercises
// the path (measured: no TLS attempt reaches the server with either). This test launches the full
// browser in its new headless mode with the mode enabled through the profile preference, and uses
// the browser's own net log as the measurement:
//   - control profile (mode off): the page loads over plain http and NO TLS handshake reaches
//     the server port, so a "loaded" result means the upgrade path did not run;
//   - HTTPS-First profile: a TLS handshake reaches the server port and fails on the certificate,
//     and the navigation is stopped with `ERR_BLOCKED_BY_CLIENT`. Measured through DevTools in
//     Playwright's Chromium (new headless), in a windowed Chrome for Testing and over
//     `connectOverCDP`: the page the tab reports is `chrome-error://chromewebdata/` holding only
//     `<html><head></head><body></body></html>` (one root accessibility node, a blank
//     screenshot), because the prompt ("This site doesn't support a secure connection", with "Go
//     back" and "Continue to site") is a browser-UI dialog, not web content. DOM, accessibility
//     tree, `Input.dispatchMouseEvent` at the button's position and keyboard input through
//     DevTools therefore never reach it; a capture of the browser window shows it, and the
//     operating system's accessibility API can invoke its button (`invokeBrowserButton`: Windows UI
//     Automation, macOS System Events, Linux AT-SPI), which the interactive test below does on
//     every platform. Only the Windows driver has been run against a real browser; the macOS and
//     Linux drivers are first exercised by the CI matrix.
//   - the certificate interstitial a direct `https://` navigation shows DOES have a DOM; the last
//     test drives its real "proceed" click path (`#details-button`, `#proceed-link`).
// A connection-level failure (refused, reset, timed out) is a dead end and fails the test.

/** What a navigation to the http address produced, plus the net-log evidence around it. */
interface Outcome {
  /** `"loaded"` or the navigation error text. */
  navigation: string;
  /** Whether a TLS handshake reached the server port. */
  tlsAttempted: boolean;
  /** Whether that handshake was rejected for its certificate. */
  certificateRejected: boolean;
  /** Whether any connection-level failure was logged for the server port. */
  connectionFailure: boolean;
}

/**
 * Navigates a fresh browser profile to `url` and reads the browser's net log for what happened on
 * `port`.
 * @param url The `http://` address to type.
 * @param port The server port whose connections are inspected.
 * @param profile A fresh profile directory (under the test output directory).
 * @param httpsFirst Whether the profile enables HTTPS-First Mode.
 * @param extraArgs Additional Chromium arguments (host resolver rules for the named project).
 * @returns The navigation result and the net-log evidence.
 * @example
 * ```
 * const outcome = await navigateWithProfile("http://192.0.2.1:32001/", "32001", "out/profile", true, []);
 * ```
 */
async function navigateWithProfile(
  url: string,
  port: string,
  profile: string,
  httpsFirst: boolean,
  extraArgs: string[],
): Promise<Outcome> {
  mkdirSync(join(profile, "Default"), { recursive: true });
  writeFileSync(
    join(profile, "Default", "Preferences"),
    JSON.stringify({ https_only_mode_enabled: httpsFirst }),
  );
  const logPath = join(profile, "netlog.json");
  let context: BrowserContext | undefined;
  let navigation = "loaded";
  try {
    context = await chromium.launchPersistentContext(profile, {
      channel: "chromium",
      args: [`--log-net-log=${logPath}`, "--net-log-capture-mode=Everything", ...extraArgs],
    });
    const page = context.pages()[0] ?? (await context.newPage());
    try {
      await page.goto(url, { waitUntil: "domcontentloaded", timeout: 20_000 });
    } catch (e) {
      navigation = String(e).split("\n")[0];
    }
  } finally {
    await context?.close();
  }
  const log = JSON.parse(readFileSync(logPath, "utf8")) as {
    constants: { logEventTypes: Record<string, number> };
    events: { type: number; source: { id: number }; params?: Record<string, unknown> }[];
  };
  const names = new Map(Object.entries(log.constants.logEventTypes).map(([k, v]) => [v, k]));
  const sources = new Set(
    log.events.filter((e) => JSON.stringify(e.params ?? {}).includes(`:${port}`)).map((e) => e.source.id),
  );
  const seen = new Set(
    log.events.filter((e) => sources.has(e.source.id)).map((e) => names.get(e.type) ?? ""),
  );
  return {
    navigation,
    tlsAttempted: seen.has("SSL_CONNECT"),
    certificateRejected: seen.has("URL_REQUEST_DELEGATE_SSL_CERTIFICATE_ERROR"),
    connectionFailure: [...seen].some((n) => /SOCKET_POOL_CONNECT_JOB_FAILED|TCP_CONNECT_ERROR/.test(n)),
  };
}

test("HTTPS-First Mode on a bare or named http address ends at the certificate interstitial, never a dead end", async ({
  baseURL,
}, testInfo) => {
  requirePrivateAddress(baseURL!, testInfo);
  const url = new URL(`${baseURL}/`);
  const args = (testInfo.project.use.launchOptions?.args ?? []).filter((a) =>
    a.startsWith("--host-resolver-rules"),
  );

  const control = await navigateWithProfile(url.href, url.port, testInfo.outputPath("profile-control"), false, args);
  expect(control.navigation, "control profile loads over plain http").toBe("loaded");
  expect(control.tlsAttempted, "control: no TLS attempt means the mode really is off").toBe(false);

  const strict = await navigateWithProfile(url.href, url.port, testInfo.outputPath("profile-strict"), true, args);
  testInfo.annotations.push({
    type: "upgrade-outcome",
    description: `${strict.navigation}; tls=${strict.tlsAttempted} cert-rejected=${strict.certificateRejected}`,
  });
  expect(strict.tlsAttempted, "HTTPS-First profile: the upgrade attempt reached the server").toBe(true);
  expect(strict.certificateRejected, "the upgraded attempt failed on the certificate").toBe(true);
  expect(strict.connectionFailure).toBe(false);
  expect(strict.navigation).toContain("ERR_BLOCKED_BY_CLIENT");
});

test("the certificate interstitial of a direct https navigation is clicked through to the app", async ({
  baseURL,
}, testInfo) => {
  test.skip(new URL(baseURL!).hostname.endsWith(".example"), "the named project maps a name; one address shape is enough");
  const url = new URL(`${baseURL}/`);
  url.protocol = "https:";
  // The full browser in its new headless mode: the default headless shell renders the error page
  // without the interstitial's DOM, so the buttons it clicks do not exist there.
  const browser = await chromium.launch({ channel: "chromium" });
  const context = await browser.newContext();
  try {
    const page = await context.newPage();
    const failure = await page.goto(url.href, { timeout: 20_000 }).then(
      () => "",
      (e: unknown) => String(e).split("\n")[0],
    );
    testInfo.annotations.push({ type: "interstitial", description: failure });
    expect(failure).toContain("ERR_CERT_");
    await page.locator("#details-button").click();
    await page.locator("#proceed-link").click();
    await expect(page.getByLabel("Username")).toBeVisible();
  } finally {
    await context.close();
    await browser.close();
  }
});

test("HTTPS-First's 'Continue to site' reaches the plain-HTTP app", async ({ baseURL }, testInfo) => {
  requirePrivateAddress(baseURL!, testInfo);
  const url = new URL(`${baseURL}/`);
  const args = (testInfo.project.use.launchOptions?.args ?? []).filter((a) => a.startsWith("--host-resolver-rules"));
  const { browser, child } = await launchWindowedChromium(testInfo.outputPath("profile-interactive"), args);
  try {
    const context = browser.contexts()[0];
    const page = context.pages()[0] ?? (await context.newPage());
    // The navigation itself fails with ERR_BLOCKED_BY_CLIENT while the prompt is up.
    await page.goto(url.href, { timeout: 20_000 }).catch(() => undefined);
    await expect
      .poll(() => invokeBrowserButton(child.pid!, "Continue to site"), {
        message: "the interstitial's 'Continue to site' button never appeared",
        timeout: 2 * INVOKE_TIMEOUT_MS,
      })
      .toBe(true);
    await expect(page.getByLabel("Username")).toBeVisible({ timeout: 20_000 });
    expect(page.url(), "Continue to site reaches the plain-HTTP origin").toMatch(/^http:\/\//);
  } finally {
    await browser.close().catch(() => undefined);
    child.kill();
  }
});
