import { test as base, expect } from "@playwright/test";
import type { Page, Browser, BrowserContext } from "@playwright/test";
import { PERFORMANCE_STORAGE_KEY } from "@shadowcat/core";

/** The full-fidelity mirror seed every e2e session boots with. The shell's boot-time auto
 * resolution reads the runner's hardware (a 4-vCPU CI runner resolves the mobile budget,
 * whose static-lighting/reduced-fidelity defaults break visual assertions written against
 * full fidelity), so the harness pins the per-device mirror — the same `localStorage` blob
 * the app's own persistence writes — before any page script runs. Product behavior is
 * untouched; this only makes the test environment deterministic. */
const PERFORMANCE_MIRROR_SEED = JSON.stringify({ preset: "quality", overrides: {} });

/** Registers `PERFORMANCE_MIRROR_SEED` (see its doc) on `context` for every page it hosts.
 * @param context The browser context to seed.
 * @example
 * ```
 * declare const context: import("@playwright/test").BrowserContext;
 * await seedPerformanceMirror(context);
 * ```
 */
export async function seedPerformanceMirror(context: BrowserContext): Promise<void> {
  await context.addInitScript(
    ([key, value]) => { window.localStorage.setItem(key, value); },
    [PERFORMANCE_STORAGE_KEY, PERFORMANCE_MIRROR_SEED],
  );
}

/** The suite's ONE context factory for sessions a spec creates beyond its default page
 * (invited players, observers): a plain `browser.newContext` plus the same
 * `seedPerformanceMirror` pin the auto fixture applies to the default context, so a
 * dual-session spec's second client runs the same deterministic budget as the first.
 * @param browser The Playwright browser to mint the context from.
 * @param options Forwarded verbatim to `browser.newContext`.
 * @returns The new context, already seeded.
 * @example
 * ```
 * declare const browser: import("@playwright/test").Browser;
 * const playerCtx = await newE2EContext(browser, { viewport: { width: 1600, height: 1000 } });
 * ```
 */
export async function newE2EContext(
  browser: Browser,
  options?: Parameters<Browser["newContext"]>[0],
): Promise<BrowserContext> {
  const context = await browser.newContext(options);
  await seedPerformanceMirror(context);
  return context;
}

/** Logs `page` in as `username`/`password` via the real login form. Shared by every spec (in
 * place of each file's own duplicated inline sequence) and by the worker `account` fixture below.
 * @param page - The page to log in.
 * @param username - The account's username.
 * @param password - The account's password.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await login(page, "ops", "pw-boot");
 * ```
 */
export async function login(
  page: Page,
  username: string,
  password: string,
): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Log in" }).click();
}

/** Idempotently opens a launcher panel by its contribution id: checks the launcher item's
 * `aria-checked` (driven by `LauncherMenu`'s live `ctx.panels.isOpen` read — the item is a
 * `role="menuitemcheckbox"` toggle, since `aria-pressed` is invalid ARIA on `role="menuitem"`)
 * and clicks only when the panel is not already open — a second call with the panel already open
 * is a no-op, unlike clicking the launcher item unconditionally (`activate`'s `ctx.panels.toggle`
 * would instead CLOSE it). Shared by every spec that reaches a launcher-closed panel (in place of
 * each file's own duplicated copy — see `combat-settings.spec.ts`'s `activateTool` for the same
 * check-before-click idiom applied to a tool-rail button).
 * @param page - The page to drive.
 * @param contributionId - The panel contribution's id (e.g. `"notes:panel"`).
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await openPanel(page, "notes:panel");
 * ```
 */
export async function openPanel(page: Page, contributionId: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  const item = page.getByTestId(`launcher-item-${contributionId}`);
  if ((await item.getAttribute("aria-checked")) !== "true") {
    await item.click();
  } else {
    // Already open: dismiss the menu we just opened via Escape rather than re-clicking the
    // trigger — the full-viewport `sc-launcher-backdrop` (`onpointerdown` dismissal) sits above
    // the trigger while the menu is open and intercepts a second trigger click, hanging the
    // action on actionability retries forever. `openMenu` already moved focus onto the first
    // item, whose own `onkeydown` handles Escape (`MenuKeyboard`'s Escape branch) identically to
    // the trigger's.
    await page.keyboard.press("Escape");
  }
}

/** Budget for a spec that drives TWO browser contexts (a GM plus an invited player) through a
 * full end-to-end scenario: world creation, account, invite, join, then the behaviour under test.
 * Sized for the slowest supported CI runner, which executes this suite at roughly four times a
 * developer machine's wall-clock — a scenario finishing in ~50s locally lands near three minutes
 * there, so a budget cut close to the local figure fails on hardware rather than on behaviour.
 *
 * INVARIANT: every dual-session spec reads this constant. Spelling the number inline splits it
 * across sites, and the split only shows up on the slowest machine that runs the suite, which is
 * never the one the number was chosen on.
 */
export const DUAL_SESSION_TIMEOUT_MS = 360_000;

/** Budget for the account-creation confirmation, sized for the contended full-suite run rather
 * than the config's `expect.timeout`: creating an account hashes the password (Argon2), and under
 * the full suite every worker mints accounts at once, so this one step contends
 * worker-count-wide — the config's own timeout comment sizes that envelope at up to ~53s per
 * test. It is a SETUP step, not an assertion about product behaviour, so covering that envelope
 * costs no coverage, while the global `expect.timeout` stays sized for genuine behavioural
 * failures.
 */
const ACCOUNT_CREATED_TIMEOUT_MS = 60_000;

/** Fills the settings panel's account form and waits for its confirmation notice.
 *
 * INVARIANT: every account-creating step in the suite routes through here, so
 * `ACCOUNT_CREATED_TIMEOUT_MS` has ONE definition. Spelling the sequence inline again splits the
 * budget across sites, which is how a contended run fails at some of them and not others.
 *
 * The caller opens the settings panel first; specs differ in when they do that.
 * @param page - The page showing the account form.
 * @param username - The account name to create.
 * @param password - The new account's password.
 * @param opts - Creation options.
 * @param opts.admin - Whether to tick "Server administrator".
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await createAccount(page, "player-0-abc", "pw-player-e2e");
 * ```
 */
export async function createAccount(
  page: Page,
  username: string,
  password: string,
  opts: {
    /** Whether to tick "Server administrator" on the creation form. */
    admin?: boolean;
  } = {},
): Promise<void> {
  await page.getByLabel("Account name").fill(username);
  await page.getByLabel("Password", { exact: true }).fill(password);
  if (opts.admin === true) await page.getByLabel("Server administrator").check();
  await page.getByRole("button", { name: "Create account" }).click();
  await expect(page.getByText(`Created account ${username}.`)).toBeVisible({
    timeout: ACCOUNT_CREATED_TIMEOUT_MS,
  });
}

/** One Playwright worker's dedicated server-admin account. */
export interface WorkerAccount {
  /** The account's username. */
  username: string;
  /** The account's password. */
  password: string;
}

/** Custom `test`, extended with a worker-scoped `account` fixture: a fresh server-admin account
 * created once per Playwright worker (not once per test, and never the shared seeded `ops`
 * account) so parallel workers stop contending on `ops`'s own `ui_state.global.lastWorld` — the
 * deeper hygiene fix behind flaky reload assertions under a full parallel run. Admin (not a plain
 * user), since some specs need admin-gated actions (creating a further throwaway account, as
 * `hex-movement.spec.ts` already does). Created via the real UI (log in as the seeded `ops`,
 * create a throwaway world to reach the Settings panel — Settings is unreachable pre-world — then
 * use the real admin-gated account-creation form), matching this suite's existing convention of
 * never bypassing the UI even for setup.
 */
export const test = base.extend<
  {
    /** The auto performance-budget pin's marker type — the fixture yields no value. */
    performanceBudgetPin: void;
  },
  {
    /** The worker-scoped account fixture — see the class doc above.
     * @example
     * ```
     * declare const account: { username: string; password: string };
     * account.username;
     * ```
     */
    account: WorkerAccount;
  }
>({
  /** Auto fixture: seeds the per-device performance mirror with the full-fidelity budget on
   * every test's default context BEFORE any page script runs (see
   * `PERFORMANCE_MIRROR_SEED`'s doc for why the runner's hardware must not decide this).
   * Contexts a spec creates itself go through `newE2EContext`, the same pin.
   * @param root0 The fixtures this setup function depends on.
   * @param root0.context The test's default browser context (covers every page it hosts).
   * @param use Playwright's fixture-provider callback.
   * @example
   * ```
   * declare const performanceBudgetPin: [
   *   (fx: { context: import("@playwright/test").BrowserContext }, use: (v: unknown) => Promise<void>) => Promise<void>,
   *   { auto: true },
   * ];
   * ```
   */
  performanceBudgetPin: [
    async ({ context }, use) => {
      await seedPerformanceMirror(context);
      await use();
    },
    { auto: true },
  ],
  /** The worker-scoped fixture value: a `[setup, options]` tuple per Playwright's
   * fixture-registration shape, `options` selecting `scope: "worker"` (see the class doc above).
   * The setup function itself mints the worker's admin account (see the class doc above) and
   * hands it to `use`.
   * @param root0 The fixtures this setup function depends on.
   * @param root0.browser The Playwright-managed browser instance, used to open a throwaway
   * context/page for the one-time account-creation flow (closed before `use` resolves).
   * @param use Playwright's fixture-provider callback; invoked once with the minted account.
   * @param workerInfo Identifies this worker (`parallelIndex`), used to make the minted
   * account's username unique per worker.
   * @returns Nothing; resolves once `use`'s callback (every test in this worker, run in
   * sequence) has completed.
   * @example
   * ```
   * declare const account: [
   *   (fx: { browser: import("@playwright/test").Browser }, use: (v: unknown) => Promise<void>, info: { parallelIndex: number }) => Promise<void>,
   *   { scope: "worker" },
   * ];
   * ```
   */
  account: [
    async ({ browser }, use, workerInfo) => {
      const suffix = `${workerInfo.parallelIndex}-${Date.now().toString(36)}`;
      const username = `e2e-worker-${suffix}`;
      const password = "pw-e2e-worker";
      const context = await newE2EContext(browser);
      const page = await context.newPage();
      await login(page, "ops", "pw-boot");
      await page.getByLabel("New world name").fill(`Worker Setup ${suffix}`);
      await page.getByRole("button", { name: "Create world" }).click();
      await page.getByTestId("launcher-trigger").click();
      await page.getByTestId("launcher-item-settings:panel").click();
      await createAccount(page, username, password, { admin: true });
      await context.close();
      await use({ username, password });
    },
    { scope: "worker" },
  ],
});

export { expect };
