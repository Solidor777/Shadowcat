import { test, expect, login, type WorkerAccount } from "./fixtures";

// The e2e server runs on `sqlite::memory:`, which has no database file to snapshot, so every
// backup route answers 422 "backup requires a file-backed database". These specs therefore
// drive the real admin UI against that real rejection: the section mounts through the settings
// section contract, the list surfaces the server's message, and Back up now reports the failure
// through the progress and notification path. Listing, restoring and deleting real backups are
// covered by the server's HTTP tests and the component tests.

/** Logs in as the worker's server admin, creates a throwaway world and opens the settings panel.
 * @param page The page to drive.
 * @param name The new world's name.
 * @param account The worker's admin account.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * declare const account: WorkerAccount;
 * await openSettings(page, "Backups World", account);
 * ```
 */
async function openSettings(
  page: import("@playwright/test").Page,
  name: string,
  account: WorkerAccount,
): Promise<void> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await page.getByTestId("topbar-settings").click();
}

test("the backup manager mounts for a server admin and shows the server's reason a list cannot be read", async ({
  page,
  account,
}) => {
  await openSettings(page, `Backups Section ${Date.now().toString(36)}`, account);

  await expect(page.getByRole("heading", { name: "Backups" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Back up now" })).toBeVisible();
  await expect(page.getByText(/Could not load backups: .*file-backed database/)).toBeVisible();
  // The one home for Back up now: the user manager no longer carries a second button.
  await expect(page.getByRole("button", { name: "Back up now" })).toHaveCount(1);
});

test("Back up now reports the server's rejection and leaves the button usable again", async ({
  page,
  account,
}) => {
  await openSettings(page, `Backups Now ${Date.now().toString(36)}`, account);

  const backUp = page.getByRole("button", { name: "Back up now" });
  await backUp.click();
  await expect(page.getByText(/Backup failed: .*file-backed database/)).toBeVisible();
  await expect(backUp).toBeEnabled();
});
