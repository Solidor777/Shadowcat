import { test, expect, login } from "./fixtures";
import { clickScene, dragScene, sceneCenter } from "./stage-gestures";

// A GM draws a circle template, selects it with the select tool and presses "Make this a
// region". The template is replaced by a region in one batch: the shape count (drawings +
// templates) drops to zero and the rail opens the region editor, which only renders for a
// region document present in the optimistic store.
test("a GM turns a drawn template into a region", async ({ page, account }) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Template Region World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const center = await sceneCenter(page);
  await page.getByTestId("tool-template").click();
  await dragScene(page, center, { x: center.x + 60, y: center.y }, 3);
  await expect(host).toHaveAttribute("data-shape-count", "1", { timeout: 15_000 });

  await page.getByTestId("tool-select").click();
  await clickScene(page, center);
  await expect(page.getByTestId("template-editor")).toBeVisible();

  await page.getByTestId("template-make-region").click();

  await expect(page.getByTestId("region-editor")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("template-editor")).toHaveCount(0);
  await expect(host).toHaveAttribute("data-shape-count", "0", { timeout: 15_000 });
});
