import { test, expect, login } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene, dragScene } from "./stage-gestures";

// Token delete e2e: with one token selected, the actors panel's quick-edit section carries a
// "Delete token" button for a viewer `ctx.canDelete` allows (a GM always). The button sends a full
// pre-image delete op, the stage's token count drops to zero and the quick-edit section goes away
// with the selection. A second test deletes the token a region follows and finds the region in the
// region tool's dormant list, where it can be removed. Observation goes through the stage host's
// `data-token-count` and the dormant list's own test ids, never WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

/** Where the token is placed and re-selected. */
const TOKEN_AT = { x: 210, y: 310 };
/** The following region's drag corners; it stays clear of the token so a click on the token
 * selects the token, not the region. */
const REGION_FROM = { x: 500, y: 100 };
const REGION_TO = { x: 800, y: 600 };
/** A point inside the region where no token stands. */
const REGION_INSIDE = { x: 650, y: 350 };

test.use({ viewport: { width: 1600, height: 1000 } });

/**
 * The stage host element of `page`.
 * @param page The page showing the stage.
 * @returns The stage host locator.
 * @example
 * ```ts
 * declare const page: import("@playwright/test").Page;
 * stageHost(page);
 * ```
 */
function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/**
 * Toggle a launcher panel open or closed.
 * @param page The GM page.
 * @param id The launcher item id, such as `actors:panel`.
 * @example
 * ```ts
 * declare const page: import("@playwright/test").Page;
 * await togglePanel(page, "actors:panel");
 * ```
 */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

/**
 * Create a world as the GM, switch grid snapping off, upload the token art, create a linked actor
 * named `Fallen` and place one token of it at `TOKEN_AT`. The actors panel is left open.
 * @param page The GM page.
 * @param account The test account's credentials.
 * @param account.username The login name.
 * @param account.password The login password.
 * @returns The actors panel locator.
 * @example
 * ```ts
 * declare const page: import("@playwright/test").Page;
 * const actors = await placeToken(page, { username: "gm", password: "pw" });
 * ```
 */
async function placeToken(page: Page, account: { username: string; password: string }): Promise<Locator> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(`Delete World ${Date.now().toString(36)}`);
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
  await actors.getByLabel("Name", { exact: true }).fill("Fallen");
  await actors.getByLabel("New independent copy on each placement").uncheck();
  await actors.getByTestId("visual-pick").click();
  const dialog = page.getByTestId("asset-pick-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("asset-tile").first().click();
  await dialog.getByTestId("pick-confirm").click();
  await expect(dialog).not.toBeVisible();
  await actors.getByRole("button", { name: "Create actor" }).click();

  await actors.getByRole("button", { name: "Fallen" }).click();
  await page.getByTestId("tool-place").click();
  await clickScene(page, TOKEN_AT);
  await expect(stageHost(page)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
  return actors;
}

test("a GM deletes the selected token from the quick-edit section", async ({ page, account }) => {
  const actors = await placeToken(page, account);

  await page.getByTestId("tool-select").click();
  await clickScene(page, TOKEN_AT);
  const quick = actors.getByTestId("token-quick-edit");
  await expect(quick).toBeVisible();

  await quick.getByTestId("token-delete").click();

  await expect(stageHost(page)).toHaveAttribute("data-token-count", "0", { timeout: 15_000 });
  await expect(actors.getByTestId("token-quick-edit")).toHaveCount(0);
});

test("deleting the token a region follows lists the region as dormant, and the list deletes it", async ({ page, account }) => {
  const actors = await placeToken(page, account);

  // A region drawn with the region tool, then attached to the token with the follow picker.
  await page.getByTestId("tool-region").click();
  await dragScene(page, REGION_FROM, REGION_TO, 4);
  await page.getByTestId("tool-select").click();
  await clickScene(page, REGION_INSIDE);
  await expect(page.getByTestId("region-editor")).toBeVisible();
  await page.getByTestId("region-follow-token").selectOption({ label: "Fallen" });
  await expect(page.getByTestId("region-follow-rotation")).toBeVisible();

  // With its carrier present the region is not dormant.
  await page.getByTestId("tool-region").click();
  await expect(page.getByTestId("region-dormant-list")).toHaveCount(0);

  // Delete the carrier from the quick-edit section.
  await page.getByTestId("tool-select").click();
  await clickScene(page, TOKEN_AT);
  await actors.getByTestId("token-quick-edit").getByTestId("token-delete").click();
  await expect(stageHost(page)).toHaveAttribute("data-token-count", "0", { timeout: 15_000 });

  // The region now has no carrier; the region tool lists it.
  await page.getByTestId("tool-region").click();
  const list = page.getByTestId("region-dormant-list");
  await expect(list).toBeVisible({ timeout: 15_000 });
  await expect(list.getByTestId("region-dormant-row")).toHaveCount(1);

  // Follow is disabled with no token selected; Delete removes the region.
  await expect(list.getByTestId("region-dormant-reanchor")).toBeDisabled();
  await list.getByTestId("region-dormant-delete").click();
  await expect(page.getByTestId("region-dormant-list")).toHaveCount(0);
});
