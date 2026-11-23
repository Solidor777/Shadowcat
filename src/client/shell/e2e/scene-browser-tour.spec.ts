import { test, expect, login, openPanel } from "./fixtures";
import type { Page } from "@playwright/test";

// The GM scene-creation tour: replayed from the settings panel (the scene browser is closed by
// default, so the tour is never offered automatically), it walks the panel, the create button and
// the scene list, then the first row's actions once a scene exists.

/** Logs `page` in and creates+enters a fresh world named `name`.
 * @param page The page to drive.
 * @param name The world's display name.
 * @param account The worker's GM account.
 */
async function enterFreshWorld(page: Page, name: string, account: { username: string; password: string }): Promise<void> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
}

test("scene-browser tour: a GM replays it from settings and is walked through creating a scene", async ({ page, account }) => {
  await enterFreshWorld(page, `Scene Tour World ${Date.now().toString(36)}`, account);

  await openPanel(page, "scene-browser:panel");
  await expect(page.getByRole("button", { name: "New scene" })).toBeVisible();

  await openPanel(page, "settings:panel");
  const replay = page.getByTestId("tour-replay-scene-browser.intro");
  await expect(replay).toBeVisible();
  await replay.click();

  const card = page.getByTestId("tour-card");
  await expect(card).toContainText("The scene browser");
  await page.getByTestId("tour-next").click();
  await expect(card).toContainText("Create a scene");
  await page.getByTestId("tour-next").click();
  await expect(card).toContainText("Scene list");
  await page.getByTestId("tour-next").click();
  await expect(card).toContainText("Use the scene");
  await page.getByTestId("tour-next").click();
  await expect(page.getByTestId("tour-overlay")).toHaveCount(0);
});
