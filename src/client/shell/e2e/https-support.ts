import type { BrowserContext, Page, TestInfo } from "@playwright/test";
import { expect, test } from "@playwright/test";
import { guardPrivateAddress } from "../e2e-support/lan-address";

/** The seeded server-admin identity every test server boots with (`SERVER_ENV` in
 * `playwright.config.ts`). */
export const OPS = { username: "ops", password: "pw-boot" } as const;

/**
 * Logs `page` in through the real login form at `origin`, waiting for the world list.
 * @param page The page to drive.
 * @param origin Scheme, host and port to sign in at (`https://…` or `http://…`).
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await loginAt(page, "https://192.0.2.1:32001");
 * ```
 */
export async function loginAt(page: Page, origin: string): Promise<void> {
  await page.goto(`${origin}/`);
  await page.getByLabel("Username").fill(OPS.username);
  await page.getByLabel("Password").fill(OPS.password);
  await page.getByRole("button", { name: "Log in" }).click();
  await expect(page.getByText("Your worlds")).toBeVisible();
}

/**
 * The plain-HTTP twin of an HTTPS origin: the server answers both schemes on one port.
 * @param httpsOrigin An `https://` origin.
 * @returns The same host and port under `http://`.
 * @example
 * ```
 * const plain = plainOrigin("https://192.0.2.1:32001"); // "http://192.0.2.1:32001"
 * ```
 */
export function plainOrigin(httpsOrigin: string): string {
  return httpsOrigin.replace(/^https:/, "http:");
}

/**
 * Status of `GET /api/me` issued from `page` itself, i.e. carrying whatever cookies the browser
 * attaches for the page origin scheme.
 * @param page A page already on the origin to probe.
 * @returns The HTTP status (200 authenticated, 401 not).
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const status = await meStatus(page);
 * ```
 */
export async function meStatus(page: Page): Promise<number> {
  return page.evaluate(async () => (await fetch("/api/me")).status);
}

/**
 * The names of the session cookies in `context`, sorted; both scheme variants end in `sc_sid`.
 * @param context The browser context whose jar is read.
 * @returns The sorted cookie names.
 * @example
 * ```
 * declare const context: import("@playwright/test").BrowserContext;
 * const names = await sessionCookieNames(context); // e.g. ["__Host-sc_sid", "sc_sid"]
 * ```
 */
export async function sessionCookieNames(context: BrowserContext): Promise<string[]> {
  return (await context.cookies())
    .filter((c) => c.name.endsWith("sc_sid"))
    .map((c) => c.name)
    .sort();
}

/** Failure text of a connection-level error, as opposed to a certificate error: a page that fails
 * this way has no browser-owned bypass, so it is a dead end for the user. */
export const CONNECTION_LEVEL_FAILURE =
  /ECONNREFUSED|CONNECTION_REFUSED|CONNECTION_RESET|CONNECTION_CLOSED|EMPTY_RESPONSE|TIMED_OUT|ERR_ADDRESS/i;

/**
 * Guards a test whose meaning depends on the address under test being private. Engines exempt
 * private (RFC 1918) addresses from upgrading `http://` to `https://`, so on a CGNAT or public
 * address every upgrade row would mean something different from its label: locally the test skips
 * with a message, on CI it fails ({@link judgeAddress}). Names (not IP literals) pass.
 * @param baseURL The project's base URL.
 * @param testInfo The running test, which receives an `address-class` annotation.
 * @example
 * ```
 * declare const testInfo: import("@playwright/test").TestInfo;
 * requirePrivateAddress("http://192.0.2.1:32001", testInfo);
 * ```
 */
export function requirePrivateAddress(baseURL: string, testInfo: TestInfo): void {
  guardPrivateAddress(baseURL, testInfo, (condition, reason) => test.skip(condition, reason), process.env.CI !== undefined);
}
