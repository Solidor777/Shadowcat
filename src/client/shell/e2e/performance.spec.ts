import { test, expect, login } from "./fixtures";
import type { Page } from "@playwright/test";

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

async function openSettings(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-settings:panel").click();
}

test("the performance editor drives the stage's frame-cap/render-scale/idle-skip signals and the statusbar readout", async ({ page, account }) => {
  const worldName = `Performance World ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await openSettings(page);
  await page.getByTestId("perf-preset-mobile").click();
  await expect(stageHost(page)).toHaveAttribute("data-fps-cap", "30");
  await expect(stageHost(page)).toHaveAttribute("data-idle-skip", "1");

  // Toggle stats: the statusbar readout appears.
  await page.getByTestId("perf-show-stats").check();
  await expect(page.getByTestId("perf-stats")).toBeVisible();
  await page.getByTestId("perf-show-stats").uncheck();
  await expect(page.getByTestId("perf-stats")).toBeHidden();

  await page.getByTestId("perf-preset-quality").click();
  await expect(stageHost(page)).toHaveAttribute("data-fps-cap", "0");
});
