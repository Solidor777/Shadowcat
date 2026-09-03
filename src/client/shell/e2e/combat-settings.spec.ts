import { test, expect, login } from "./fixtures";
import type { Page } from "@playwright/test";

const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

// Scene-space geometry: the camera starts at its default offset (0,0) scale 1, so a scene
// coordinate is exactly a canvas-local pixel. This scene is left at the default square grid
// (size 100) — the movement-budget gate is grid-shape-agnostic (it counts cells, not geometry),
// so a square grid keeps the arithmetic below simple and independent of any hex axial math.
const TOKEN_Y = 300;
const PLACE_X = 200;
const HANDOFF_X = 250;
/** Three cells past the handoff point — a budget of 2 cells covers the first two, truncating the
 * third under `hard` enforcement. */
const TARGET_X = HANDOFF_X + 300;
/** The cell the truncated move must land on: exactly `movement`'s budget (2 cells × grid size
 * 100) past the handoff's own cell start. */
const TRUNCATED_X = HANDOFF_X + 200;

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

async function openGameSettings(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-game-settings:panel").click();
}

async function closeGameSettings(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-game-settings:panel").click();
}

async function dragScene(
  page: Page,
  from: { x: number; y: number },
  to: { x: number; y: number },
): Promise<void> {
  const box = await page.getByTestId("stage-canvas").boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + from.x, box!.y + from.y);
  await page.mouse.down();
  await page.mouse.move(box!.x + to.x, box!.y + to.y);
  await page.mouse.up();
}

test("the resource registry and combat chain editors drive a real movement-budget gate", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(180_000);
  const playerName = `player-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Combat Settings World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();
  await gm.getByLabel("Account name").fill(playerName);
  await gm.getByLabel("Password", { exact: true }).fill(playerPassword);
  await gm.getByRole("button", { name: "Create account" }).click();
  await expect(gm.getByText(`Created account ${playerName}.`)).toBeVisible({ timeout: 15_000 });
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();

  const playerCtx = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await openGameSettings(gm);

    // --- Resources editor: add a Tracked "movement" resource. `max`/`turnStart` carry a plain
    // numeric-literal formula rather than a system-leaf reference — `SystemTreeEditor` has no
    // key-rename affordance (an added field's key is a random opaque id with no rename
    // control), so authoring a NAMED `system.speed` leaf through that editor is not achievable
    // today; the numeric literal exercises the identical movement-budget gate
    // (Resource -> CombatDefaults -> execute_move) without depending on that unrelated gap.
    await gm.getByLabel("gameSettings.resources.key").fill("movement");
    await gm.getByRole("button", { name: "gameSettings.resources.add" }).click();
    await gm.getByLabel("gameSettings.resources.kind-movement").selectOption("tracked");
    await gm.getByLabel("gameSettings.resources.max-movement").fill("2");
    await gm.getByLabel("gameSettings.resources.turnStart-movement").fill("2");

    // --- Chain editor (world tier): movementResource = movement, interpretation = per_cell,
    // enforcement = hard.
    await gm.getByLabel("gameSettings.combat.movementResource").selectOption("movement");
    await expect(gm.getByTestId("provenance:combat.movementResource")).toHaveText("gameSettings.source.world");
    await gm.getByLabel("gameSettings.combat.interpretation").selectOption("per_cell");
    await gm.getByLabel("gameSettings.combat.enforcement").selectOption("hard");
    await expect(gm.getByTestId("provenance:combat.enforcement")).toHaveText("gameSettings.source.world");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"hard"');

    // --- Scene tier: enforcement = warn overrides the world's hard on the selected scene.
    await gm.getByLabel("gameSettings.combat.scene.enforcement").selectOption("warn");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"warn"');
    await expect(gm.getByTestId("provenance:combat.scene.enforcement")).toHaveText("gameSettings.source.scene");

    // Reset the scene override — falls back to the world's hard.
    await gm.getByLabel("gameSettings.combat.scene.enforcement").selectOption("__inherit");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"hard"');

    // World reset — falls all the way back to the engine default (none) — then re-arm hard for
    // the gate proof below.
    await gm.getByLabel("gameSettings.combat.enforcement").selectOption("__inherit");
    await expect(gm.getByTestId("provenance:combat.enforcement")).toHaveText("gameSettings.source.engine");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"none"');
    await gm.getByLabel("gameSettings.combat.enforcement").selectOption("hard");
    await closeGameSettings(gm);

    // --- Seat the player's token: art, placement, ownership.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await gm.getByTestId("tool-place").click();
    await gm.getByTestId("picker-asset").first().click();
    let box = await gm.getByTestId("stage-canvas").boundingBox();
    expect(box).not.toBeNull();
    await gm.mouse.click(box!.x + PLACE_X, box!.y + TOKEN_Y);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();

    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    box = await gm.getByTestId("stage-canvas").boundingBox();
    await gm.mouse.click(box!.x + PLACE_X, box!.y + TOKEN_Y);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });
    await gm.mouse.move(box!.x + 150, box!.y + 250);
    await gm.mouse.down();
    await gm.mouse.move(box!.x + 250, box!.y + 350);
    await gm.mouse.up();
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    // --- Combat: create, add the selected token, start.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();
    await gm.getByTestId("combat-tracker:create").click();
    await expect(gm.getByTestId("combat-tracker:add-selected")).toBeEnabled({ timeout: 15_000 });
    await gm.getByTestId("combat-tracker:add-selected").click();
    await gm.getByRole("button", { name: "combatTracker.start" }).click();
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();

    // Happens-before barrier: the GM's own handoff nudge, observed by the player, precedes every
    // player drag below (`hex-movement.spec.ts`'s pattern — frames arrive in sequence order on
    // one socket).
    await dragScene(gm, { x: PLACE_X, y: TOKEN_Y }, { x: HANDOFF_X, y: TOKEN_Y });
    await player.getByTestId("tool-select").click();
    await expect
      .poll(async () => (await stageHost(player).getAttribute("data-token-positions")) ?? "", { timeout: 20_000 })
      .toContain(":" + HANDOFF_X + ",");

    // --- Gate proof, hard enforcement: a drag spanning 3 cells against a 2-cell budget
    // truncates at the 2-cell mark. `Stage`'s `data-last-move-outcome` mirrors the server's own
    // move-resolution outcome directly, independent of any canvas-rendered preview text.
    await dragScene(player, { x: HANDOFF_X, y: TOKEN_Y }, { x: TARGET_X, y: TOKEN_Y });
    await expect(stageHost(player)).toHaveAttribute("data-last-move-outcome", "truncated", { timeout: 20_000 });
    await expect
      .poll(async () => (await stageHost(gm).getAttribute("data-token-positions")) ?? "", { timeout: 20_000 })
      .toContain(":" + TRUNCATED_X + ",");

    // The tracker's row shows the decremented `current` for the movement resource after the
    // truncated move consumed its full budget.
    const resourceCell = gm.locator('[data-testid="combat-tracker:resource-movement"]');
    await expect(resourceCell).toContainText("0", { timeout: 15_000 });

    // --- Re-authoring to warn: the overage executes in full instead of truncating.
    await openGameSettings(gm);
    await gm.getByLabel("gameSettings.combat.enforcement").selectOption("warn");
    await closeGameSettings(gm);

    // Refill the budget for a clean second measurement (advance the turn cycle back to the
    // player's own turn so `turnStart` recovery re-applies `movement`'s max).
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();
    for (let i = 0; i < 3; i++) {
      const current = await resourceCell.textContent();
      if (current && current.trim() !== "0") break;
      await gm.getByTestId("combat-tracker:advance").click();
    }

    await dragScene(player, { x: TRUNCATED_X, y: TOKEN_Y }, { x: TRUNCATED_X + 300, y: TOKEN_Y });
    await expect(stageHost(player)).toHaveAttribute("data-last-move-outcome", "executed", { timeout: 20_000 });
  } finally {
    await playerCtx.close();
  }
});
