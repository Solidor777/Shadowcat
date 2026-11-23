import { test, expect, login } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Token nameplate and attribute-bar authoring e2e: the GM authors an actor with a nameplate mode
// and one bar through the actors panel, places a linked token, then switches the token to its own
// bar list through the per-token override control. The drawn plate is canvas output with no DOM
// observable, so the test covers the authoring surfaces and their persistence (the editors
// re-read the stored documents), not pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

const PLACE_AT = { x: 1010, y: 510 };

test.use({ viewport: { width: 1600, height: 1000 } });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("a GM authors actor bars and a per-token bar override that persist", async ({ page, account }) => {
  const worldName = `Bars World ${Date.now().toString(36)}`;
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const snap = page.getByTestId("snap-toggle");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");

  await togglePanel(page, "asset-browser:panel");
  await page.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);
  await togglePanel(page, "asset-browser:panel");

  await togglePanel(page, "actors:panel");
  const actors = page.locator(".actors");
  await actors.getByLabel("Name", { exact: true }).fill("Brute");
  await actors.getByLabel("New independent copy on each placement").uncheck();
  await actors.getByTestId("visual-pick").click();
  const dialog = page.getByTestId("asset-pick-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("asset-tile").first().click();
  await dialog.getByTestId("pick-confirm").click();
  await expect(dialog).not.toBeVisible();

  // The create form's own bar editor: one bar, always shown, with the nameplate always shown.
  const form = actors.locator("form");
  await form.getByTestId("create-nameplate-mode").selectOption("always");
  await form.getByTestId("bar-add").click();
  await form.getByTestId("bar-current").fill("7");
  await form.getByTestId("bar-current").press("Tab");
  await form.getByTestId("bar-max").fill("10");
  await form.getByTestId("bar-max").press("Tab");
  await form.getByTestId("bar-mode").selectOption("always");
  await actors.getByRole("button", { name: "Create actor" }).click();

  // The row editor reads the stored values back.
  const row = actors.getByRole("listitem");
  await expect(row).toHaveCount(1);
  await expect(row.getByTestId("actor-nameplate-mode")).toHaveValue("always");
  await expect(row.getByTestId("bar-current")).toHaveValue("7");
  await expect(row.getByTestId("bar-max")).toHaveValue("10");
  await expect(row.getByTestId("bar-mode")).toHaveValue("always");

  // Place a linked token and select it; the per-token control appears and inherits by default.
  await row.getByRole("button", { name: "Brute" }).click();
  await page.getByTestId("tool-place").click();
  await clickScene(page, PLACE_AT);
  await expect(stageHost(page)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
  await page.getByTestId("tool-select").click();
  await clickScene(page, PLACE_AT);
  const source = actors.getByTestId("token-bars-source");
  await expect(source).toHaveValue("inherit");

  // Switching to a custom list seeds it from the actor's; editing it leaves the actor untouched.
  await source.selectOption("custom");
  const tokenBars = actors.locator(".token-bars").getByTestId("bar-current");
  await expect(tokenBars).toHaveValue("7");
  await tokenBars.fill("3");
  await tokenBars.press("Tab");
  await expect(tokenBars).toHaveValue("3");
  await expect(row.getByTestId("bar-current")).toHaveValue("7");
});
