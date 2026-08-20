import { test as base, expect } from "@playwright/test";
import type { Page } from "@playwright/test";

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
  Record<string, never>,
  { account: WorkerAccount }
>({
  account: [
    async ({ browser }, use, workerInfo) => {
      const suffix = `${workerInfo.parallelIndex}-${Date.now().toString(36)}`;
      const username = `e2e-worker-${suffix}`;
      const password = "pw-e2e-worker";
      const context = await browser.newContext();
      const page = await context.newPage();
      await login(page, "ops", "pw-boot");
      await page.getByLabel("New world name").fill(`Worker Setup ${suffix}`);
      await page.getByRole("button", { name: "Create world" }).click();
      await page.getByTestId("launcher-trigger").click();
      await page.getByTestId("launcher-item-settings:panel").click();
      await page.getByLabel("Account name").fill(username);
      await page.getByLabel("Password", { exact: true }).fill(password);
      await page.getByLabel("Server administrator").check();
      await page.getByRole("button", { name: "Create account" }).click();
      await expect(page.getByText(`Created account ${username}.`)).toBeVisible({
        timeout: 15_000,
      });
      await context.close();
      await use({ username, password });
    },
    { scope: "worker" },
  ],
});

export { expect };
