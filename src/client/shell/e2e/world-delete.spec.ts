import { expect, test, type Page } from "@playwright/test";

async function login(page: Page, username: string, password: string): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Log in" }).click();
}

test("world delete: type-the-name confirm removes the world server-side", async ({ page }) => {
  // Unique name: the dev server persists across local runs (reuseExistingServer).
  const worldName = `Delete Me ${Date.now().toString(36)}`;
  await login(page, "ops", "pw-boot");
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.getByTestId("stage-canvas")).toBeVisible();

  // Back to the world list via the settings panel's Leave world — a bare
  // goto("/") re-enters the persisted last world instead of showing the roster.
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("button", { name: "Leave world", exact: true }).click();
  const row = page.locator("li", { hasText: worldName });
  await row.getByRole("button", { name: "Delete", exact: true }).click();
  await row.getByLabel("Type the world name to confirm deletion").fill(worldName);
  await row.getByRole("button", { name: "Delete forever" }).click();
  await expect(page.getByRole("button", { name: new RegExp(worldName) })).toHaveCount(0);

  // Server-side, not just local state: the row stays gone after a reload.
  await page.reload();
  await expect(page.getByRole("button", { name: new RegExp(worldName) })).toHaveCount(0);
});
