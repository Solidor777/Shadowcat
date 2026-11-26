import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene, dragScene } from "./stage-gestures";

// Fog-region e2e: a GM draws a region that blocks sight and light with the region tool's sense
// checkboxes, and the observing player's lit area (the stage's read-only `data-lit-bbox`,
// `minI,minJ,maxI,maxJ`) shrinks to the cells before the region's edge. Clearing both
// checkboxes in the region editor restores it. Authoring goes through the real UI; observation
// goes through the debug attribute, never WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Scene-space geometry (square grid, 100-unit cells, snap OFF so a click authors exactly that
// position). The default scene is pitch dark, so the only light is the player's own carried
// torch (`DEFAULT_LIGHT_EMISSION`: a 600-unit dim reach, committed lit set out to ~464 units).
const BEARER = { x: 210, y: 310 };
/** The fog region's drag corners. Its west edge (x = 400) lies between the torch and the cells
 * east of it; its rows span every row the torch can light, so no ray slips past its top or
 * bottom. */
const FOG_FROM = { x: 400, y: 20 };
const FOG_TO = { x: 1000, y: 760 };
/** A point inside the region where no token stands, used to select it with the select tool. */
const FOG_INSIDE = { x: 700, y: 400 };
/** The open scene lights columns past the region's west edge (the torch reaches column 6). */
const OPEN_MIN_MAX_COL = 5;
/** With the fog drawn, only columns west of the edge (0..3) can be lit. */
const FOGGED_MAX_COL = 3;

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** The eastmost lit column of a `data-lit-bbox` value (`minI,minJ,maxI,maxJ`), or null. */
function maxCol(bbox: string): number | null {
  if (!bbox) return null;
  const n = Number(bbox.split(",")[2]);
  return Number.isFinite(n) ? n : null;
}

/** Turn grid snapping off so every authored point is exactly the clicked scene coordinate. */
async function disableSnap(page: Page): Promise<void> {
  const snap = page.getByTestId("snap-toggle");
  await expect(snap).toHaveAttribute("aria-pressed", "true");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");
}

/** Upload the 1×1 PNG through the assets panel (real upload pipeline), then close the panel. */
async function uploadTokenArt(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();
  await page.getByTestId("asset-upload-input").setInputFiles({
    name: "tok.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
}

/** Choose the uploaded art for the actor create form through the asset pick dialog. */
async function pickActorArt(page: Page): Promise<void> {
  await page.locator(".actors").getByTestId("visual-pick").click();
  const dialog = page.getByTestId("asset-pick-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("asset-tile").first().click();
  await dialog.getByTestId("pick-confirm").click();
  await expect(dialog).not.toBeVisible();
}

test("a sight and light fog region shrinks the player's lit area and clearing it restores it", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const gm = page;
  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Fog World ${Date.now().toString(36)}`;

  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await newE2EContext(browser, {
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // The member roster is a session-start snapshot: the GM re-enters the world so the seated
    // player appears in the owner select.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // The docked settings and chat panels narrow the canvas past the fog region's far corner.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-settings:panel").click();
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-chat:panel").click();

    await disableSnap(gm);
    await uploadTokenArt(gm);

    // A linked, player-owned actor carrying a torch, placed at BEARER.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    const actors = gm.locator(".actors");
    await actors.getByLabel("New independent copy on each placement").uncheck();
    await actors.getByLabel("Name", { exact: true }).fill("Bearer");
    await pickActorArt(gm);
    await actors.getByRole("button", { name: "Create actor" }).click();
    const bearer = actors.getByRole("listitem").filter({ hasText: "Bearer" });
    await expect(bearer).toHaveCount(1);
    await bearer.getByLabel("Owner").selectOption({ label: playerName });
    await bearer.getByLabel("Carried light").check();
    await expect(bearer.getByLabel("Carried light")).toBeChecked();
    await bearer.getByRole("button", { name: "Bearer", exact: true }).click();
    await gm.getByTestId("tool-place").click();
    await clickScene(gm, BEARER);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await expect(stageHost(player)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    const litMaxCol = async (): Promise<number | null> =>
      maxCol((await stageHost(player).getAttribute("data-lit-bbox")) ?? "");

    // Control: with no fog, the torch lights columns east of where the region will stand.
    await expect
      .poll(async () => (await litMaxCol()) ?? -1, {
        message: "the open scene must light columns past the future fog edge",
        timeout: 20_000,
      })
      .toBeGreaterThanOrEqual(OPEN_MIN_MAX_COL);

    // Draw the fog with the region tool: tick sight and light, then drag the rectangle.
    await gm.getByTestId("tool-region").click();
    await gm.getByTestId("region-occludes-sight").check();
    await gm.getByTestId("region-occludes-light").check();
    await dragScene(gm, FOG_FROM, FOG_TO, 4);

    await expect
      .poll(async () => (await litMaxCol()) ?? -1, {
        message: "the fog region must cut the player's lit area at its west edge",
        timeout: 20_000,
      })
      .toBeLessThanOrEqual(FOGGED_MAX_COL);

    // Clear both senses in the region editor: the region stays but no longer occludes.
    await gm.getByTestId("tool-select").click();
    await clickScene(gm, FOG_INSIDE);
    await expect(gm.getByTestId("region-editor")).toBeVisible();
    await gm.getByTestId("region-editor-occludes-sight").uncheck();
    await gm.getByTestId("region-editor-occludes-light").uncheck();

    await expect
      .poll(async () => (await litMaxCol()) ?? -1, {
        message: "clearing the senses must restore the lit area past the region's edge",
        timeout: 20_000,
      })
      .toBeGreaterThanOrEqual(OPEN_MIN_MAX_COL);
  } finally {
    await playerCtx.close();
  }
});
