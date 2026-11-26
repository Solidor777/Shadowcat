import { test, expect, login } from "./fixtures";

// The GM's topbar on a 360 px phone: the pause controls share the row with the launcher, the title,
// the presence roster and the settings entry, and the settings entry must stay on screen with no
// horizontal page overflow.

const PHONE = { width: 360, height: 740 };
test.use({ viewport: PHONE });

test("the GM's topbar fits a 360 px phone and keeps the settings entry on screen", async ({ page, account }) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(`Phone Topbar World ${Date.now().toString(36)}`);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const pause = page.getByTestId("topbar-pause-toggle");
  const settings = page.getByTestId("topbar-settings");
  await expect(pause).toBeVisible();
  await expect(settings).toBeVisible();

  const settingsBox = await settings.boundingBox();
  const pauseBox = await pause.boundingBox();
  const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth);
  test.info().annotations.push({
    type: "measured",
    description: JSON.stringify({ settingsRight: settingsBox && settingsBox.x + settingsBox.width, pauseBox, scrollWidth }),
  });

  expect(settingsBox).not.toBeNull();
  expect(settingsBox!.x + settingsBox!.width).toBeLessThanOrEqual(PHONE.width);
  expect(pauseBox!.x + pauseBox!.width).toBeLessThanOrEqual(PHONE.width);
  expect(scrollWidth).toBeLessThanOrEqual(PHONE.width);
  await settings.click();
  await expect(page.getByTestId("topbar-settings")).toBeVisible();
});
