import { test, expect, login } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Token quick-edit e2e: with one token selected, the actors panel's quick-edit section holds the
// elevation control, the condition palette and one nudge row per delivered bar. A bar whose
// `current` is a bare reference to a `system` leaf writes that leaf through the ordinary Update
// path, so the actor sheet reads the nudged value back. The drawn bar is canvas output with no
// DOM observable, so the test checks the stored leaf, not pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// In canvas-local pixels, inside the canvas: the docked actors panel narrows the canvas, so a point
// past its right edge lands on the panel instead.
const PLACE_AT = { x: 380, y: 460 };

test.use({ viewport: { width: 1600, height: 1000 } });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("a GM nudges a bar leaf, raises elevation and toggles a condition from the quick-edit section", async ({ page, account }) => {
  const worldName = `Quick Edit World ${Date.now().toString(36)}`;
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

  // An actor with one always-shown bar whose `current` names the `system.hp` leaf.
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
  const form = actors.locator("form");
  await form.getByTestId("bar-add").click();
  await form.getByTestId("bar-current").fill("hp");
  await form.getByTestId("bar-current").press("Tab");
  await form.getByTestId("bar-max").fill("10");
  await form.getByTestId("bar-max").press("Tab");
  await form.getByTestId("bar-mode").selectOption("always");
  await actors.getByRole("button", { name: "Create actor" }).click();

  // The `hp` leaf, authored through the actor sheet's `SystemTreeEditor`.
  await actors.getByRole("button", { name: "Open sheet" }).click();
  const sheet = page.getByRole("dialog", { name: "Sheet", exact: true });
  await expect(sheet).toBeVisible();
  await sheet.getByText("All data").click();
  await sheet.getByLabel("New field key").fill("hp");
  await sheet.getByLabel("New field value").fill("7");
  await sheet.getByRole("button", { name: "Add field" }).click();
  await expect(sheet.getByLabel("hp", { exact: true })).toHaveValue("7");
  await sheet.getByRole("button", { name: "Close" }).click();

  // Place a linked token and select it; the quick-edit section appears.
  await actors.getByRole("button", { name: "Brute" }).click();
  await page.getByTestId("tool-place").click();
  await clickScene(page, PLACE_AT);
  await expect(stageHost(page)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
  await page.getByTestId("tool-select").click();
  await clickScene(page, PLACE_AT);
  const quick = actors.getByTestId("token-quick-edit");
  await expect(quick).toBeVisible();

  // The bar row shows the delivered value; lowering it writes the leaf.
  const value = quick.getByTestId("quick-bar-value");
  await expect(value).toHaveValue("7");
  await quick.getByTestId("quick-bar-dec").click();
  await expect(value).toHaveValue("6");

  // Elevation lives in the same section.
  const elevation = quick.getByTestId("token-elevation");
  await elevation.fill("3");
  await elevation.press("Tab");
  await expect(elevation).toHaveValue("3");

  // A condition chip toggles from the section's palette.
  const chip = quick.locator("button[aria-pressed]").first();
  await expect(chip).toHaveAttribute("aria-pressed", "false");
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "true");

  // The nudge reached the actor's stored leaf: the sheet reads 6 back.
  await actors.getByRole("button", { name: "Open sheet" }).click();
  const reopened = page.getByRole("dialog", { name: "Sheet", exact: true });
  await reopened.getByText("All data").click();
  await expect(reopened.getByLabel("hp", { exact: true })).toHaveValue("6");
});
