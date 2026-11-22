import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Facing-limited vision e2e: with the world switch on and the token's actor carrying a narrow
// forward arc, the player's lit region sits on the side the token faces; the player then turns
// the token through the rotation control and the lit region moves to the new side. Observation
// goes through the stage's read-only `data-lit-bbox` attribute ("minI,minJ,maxI,maxJ"), never
// WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Snap is off so the click lands exactly here, mid-scene so both sides have room to be lit.
const WATCHER = { x: 1010, y: 510 };

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** The `[minI, minJ, maxI, maxJ]` lit box of a page's stage, or null while nothing is lit. */
async function litBox(page: Page): Promise<number[] | null> {
  const raw = await stageHost(page).getAttribute("data-lit-bbox");
  if (!raw) return null;
  return raw.split(",").map(Number);
}

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("a player turning a facing-limited token moves the lit region to the side it faces", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Facing World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await togglePanel(gm, "settings:panel");
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

    // The member roster is a session-start snapshot: re-enter so the player appears as an owner.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    const snap = gm.getByTestId("snap-toggle");
    await expect(snap).toHaveAttribute("aria-pressed", "true");
    await snap.click();
    await expect(snap).toHaveAttribute("aria-pressed", "false");

    await togglePanel(gm, "asset-browser:panel");
    await gm.getByTestId("asset-upload-input").setInputFiles({
      name: "tok.png",
      mimeType: "image/png",
      buffer: PNG_1X1,
    });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await togglePanel(gm, "asset-browser:panel");

    // Turn the world's facing-vision switch on.
    await togglePanel(gm, "game-settings:panel");
    await gm.getByLabel("Facing-limited vision").check();
    await togglePanel(gm, "game-settings:panel");

    // A linked, player-owned actor with a 90-degree forward arc and no all-round awareness.
    await togglePanel(gm, "actors:panel");
    const actors = gm.locator(".actors");
    await actors.getByLabel("Name", { exact: true }).fill("Watcher");
    await actors.getByLabel("New independent copy on each placement").uncheck();
    await actors.getByTestId("visual-pick").click();
    const dialog = gm.getByTestId("asset-pick-dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByTestId("asset-tile").first().click();
    await dialog.getByTestId("pick-confirm").click();
    await expect(dialog).not.toBeVisible();
    await actors.getByRole("button", { name: "Create actor" }).click();
    const row = actors.getByRole("listitem");
    await expect(row).toHaveCount(1);
    await row.getByLabel("Owner").selectOption({ label: playerName });

    // Sight for the actor, so the dark default scene lights what it perceives.
    await row.getByTestId("vision-add").click();
    await row.getByTestId("vision-mode-0").selectOption({ index: 0 });
    await row.getByTestId("vision-range-0").fill("12");
    await row.getByTestId("vision-range-0").press("Tab");

    // The facing arc: enable, then narrow to 90 degrees with no near radius.
    await row.getByTestId("facing-enabled").check();
    await row.getByTestId("facing-arc").fill("90");
    await row.getByTestId("facing-arc").press("Tab");
    await row.getByTestId("facing-near").fill("0");
    await row.getByTestId("facing-near").press("Tab");

    await row.getByRole("button", { name: "Watcher" }).click();
    await gm.getByTestId("tool-place").click();
    await clickScene(gm, WATCHER);
    await expect(stageHost(player)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

    // The token faces +x (rotation 0): the lit region lies east of the token, none west of it.
    await expect
      .poll(async () => (await litBox(player)) !== null, {
        message: "the player's stage must have painted a lit region",
        timeout: 20_000,
      })
      .toBe(true);
    const faced = (await litBox(player))!;
    const tokenCol = Math.floor(WATCHER.x / 100);
    expect(faced[2]).toBeGreaterThan(tokenCol);
    expect(faced[0]).toBeGreaterThanOrEqual(tokenCol);

    // The player selects the token and turns it half a circle through the rotation control.
    await player.getByTestId("tool-select").click();
    await clickScene(player, WATCHER);
    const rotation = player.getByLabel("Rotation (degrees)");
    await expect(rotation).toBeVisible();
    await rotation.fill("180");
    await rotation.press("Tab");

    // The fog hole rotates with it: now the lit region lies west of the token and none east.
    await expect
      .poll(async () => (await litBox(player))?.[0] ?? tokenCol, {
        message: "the lit region must move west once the token faces west",
        timeout: 20_000,
      })
      .toBeLessThan(tokenCol);
    const turned = (await litBox(player))!;
    expect(turned[2]).toBeLessThanOrEqual(tokenCol);
  } finally {
    await playerCtx.close();
  }
});
