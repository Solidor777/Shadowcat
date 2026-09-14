import { test, expect, login, createAccount, newE2EContext, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page } from "@playwright/test";
import { clickScene } from "./stage-gestures";
import type { ScenePoint } from "./stage-gestures";

// A 1×1 PNG used as token art (same fixture `hex-movement.spec.ts`/`assets.spec.ts` use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// A 4-frame, 8×8 lossless animated WebP (100ms/frame; frame i paints rows 2i/2i+1 red/blue),
// uploaded as an in-memory buffer — mirrors asset-browser.spec.ts's PNG_1X1 convention.
const ANIMATED_WEBP_4F = Buffer.from(
  "UklGRuwAAABXRUJQVlA4WAoAAAACAAAABwAABwAAQU5JTQYAAAAAAAAAAABBTk1GKgAAAAAAAAAAAAcAAAEAAGQAAANWUDhMEgAAAC8HQAAADxDzv//zHw7mP6L/EUFOTUYqAAAAAAAAAQAABwAAAQAAZAAAAVZQOEwSAAAALwdAAAAPEPO///MfDuY/ov8RQU5NRioAAAAAAAACAAAHAAABAABkAAABVlA4TBIAAAAvB0AAAA8Q87//8x8O5j+i/xFBTk1GKgAAAAAAAAMAAAcAAAEAAGQAAABWUDhMEgAAAC8HQAAADxDzv//zHw7mP6L/EQ==",
  "base64",
);

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

/** Activates the place tool, picks the first asset once, then places one raw token per
 * point (the place tool keeps the picked asset across placements — same helper shape as
 * `combat-tracker.spec.ts`'s own).
 * @param page The GM's page.
 * @param points Canvas-local points to place a token at, in order.
 */
async function placeTokens(page: Page, points: readonly ScenePoint[]): Promise<void> {
  await page.getByTestId("tool-place").click();
  const pick = page.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();
  for (const p of points) await clickScene(page, p);
}

// Two browser contexts (GM + invited player), the `combat-tracker.spec.ts` seating flow.
// The player (never the GM) fires the FX tool's one-shot — any world member may fire one
// per the server's `vfx_permitted` authz, so the player path is the coverage that matters.
test("emitter playback and the FX scene tool are visible to both GM and player; a player's vfx toggle is local-only", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `VFX World ${Date.now().toString(36)}`;

  const gm = page;
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

  // Token art, then the animated effect — the effect asset additionally takes the
  // explicit `vfx` tag, which the FX tool's asset picker filters on.
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-asset-browser:panel").click();
  await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
  await gm.getByTestId("asset-upload-input").setInputFiles({ name: "anim.webp", mimeType: "image/webp", buffer: ANIMATED_WEBP_4F });
  await expect(gm.getByTestId("asset-tile")).toHaveCount(2);
  await gm.getByTestId("asset-tile").first().click();
  await gm.getByTestId("preview-tag-input").fill("vfx");
  await gm.getByTestId("preview-tag-input").press("Enter");
  await expect(gm.getByTestId("preview-tag-remove-vfx")).toBeVisible();

  // A raw token carrying the emission: place it, select it, author the VFX override
  // through the actors panel's per-token emission control.
  await placeTokens(gm, [{ x: 200, y: 300 }]);
  await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
  await gm.getByTestId("tool-select").click();
  await clickScene(gm, { x: 200, y: 300 });
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-actors:panel").click();
  const emissions = gm.locator(".token-emissions");
  await emissions.getByLabel("VFX", { exact: true }).check();
  await emissions.getByLabel("VFX asset").selectOption({ label: "anim.webp" });

  // The emitter reaches the GM's own stage immediately.
  await expect(stageHost(gm)).toHaveAttribute("data-vfx-count", "1", { timeout: 15_000 });

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

    // The emitter plays for the invited player too (the broadcast document set plus a
    // metadata warm, never a second asset pipeline).
    await expect(stageHost(player)).toHaveAttribute("data-vfx-count", "1", { timeout: 15_000 });

    // The PLAYER fires the FX tool's one-shot: pick the tagged effect once via the FX
    // panel, then click the stage. The one-shot broadcasts room-wide (both counts rise).
    await player.getByTestId("launcher-trigger").click();
    await player.getByTestId("launcher-item-vfx:panel").click();
    await player.getByTestId("fx-pick-effect").click();
    await player.getByTestId("asset-tile").first().click();
    await expect(player.getByTestId("fx-preview")).toBeVisible({ timeout: 10_000 });
    await player.getByTestId("scene-tool-vfx").click();
    await clickScene(player, { x: 600, y: 300 });
    await expect(stageHost(player)).toHaveAttribute("data-vfx-count", "2", { timeout: 15_000 });
    await expect(stageHost(gm)).toHaveAttribute("data-vfx-count", "2", { timeout: 15_000 });

    // The player's own per-device `vfx` budget toggle (Settings → Performance) suppresses
    // playback locally only — the GM's stage is unaffected (a client-local performance
    // setting, never a server-side suppression).
    await player.getByTestId("launcher-trigger").click();
    await player.getByTestId("launcher-item-settings:panel").click();
    await player.getByTestId("perf-vfx").uncheck();
    await expect(stageHost(player)).toHaveAttribute("data-vfx-count", "0", { timeout: 15_000 });
    await expect(stageHost(gm)).toHaveAttribute("data-vfx-count", "2");
  } finally {
    await playerCtx.close();
  }
});
