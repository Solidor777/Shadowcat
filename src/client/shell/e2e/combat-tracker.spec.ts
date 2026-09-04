import { test, expect, login } from "./fixtures";
import type { Page } from "@playwright/test";

// A 1×1 PNG used as token art (same fixture `hex-movement.spec.ts`/`assets.spec.ts` use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

async function openTracker(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-combat-tracker:panel").click();
}

async function closeTracker(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-combat-tracker:panel").click();
}

async function placeToken(page: Page, x: number, y: number): Promise<void> {
  await page.getByTestId("tool-place").click();
  const pick = page.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();
  const box = await page.getByTestId("stage-canvas").boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.click(box!.x + x, box!.y + y);
}

// Two browser contexts (GM + invited player), the `hex-movement.spec.ts` seating flow: the GM
// creates the world/player account/invite, the player redeems it, and the GM re-enters so the
// freshly seated player is assignable as a token owner (AppContext's member roster is a
// session-start snapshot). Everything the tracker does afterward — turn advance, hide/reveal,
// rewind, end — is asserted on BOTH browsers, since the server is the sole authority for what
// each side ever sees.
test("the combat tracker runs a full turn cycle across a GM and player session", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(180_000);

  const playerName = `player-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Combat Tracker World ${Date.now().toString(36)}`;

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

    // Re-enter as the GM so the freshly seated player is assignable as a token owner.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // Token art, then two tokens: one for the player, one an unassigned NPC.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);

    await placeToken(gm, 200, 300);
    await placeToken(gm, 400, 300);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();

    // Assign the first token's owner to the player via the actors panel.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    const canvasBox = await gm.getByTestId("stage-canvas").boundingBox();
    expect(canvasBox).not.toBeNull();
    await gm.mouse.click(canvasBox!.x + 200, canvasBox!.y + 300);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });

    // Select both tokens (drag-select) for the tracker's "add selected".
    const start = { x: canvasBox!.x + 150, y: canvasBox!.y + 250 };
    const end = { x: canvasBox!.x + 450, y: canvasBox!.y + 350 };
    await gm.mouse.move(start.x, start.y);
    await gm.mouse.down();
    await gm.mouse.move(end.x, end.y);
    await gm.mouse.up();
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    // Open the tracker; create the combat and add both selected tokens plus an event.
    await openTracker(gm);
    await gm.getByTestId("combat-tracker:create").click();
    await expect(gm.getByTestId("combat-tracker:add-selected")).toBeEnabled({ timeout: 15_000 });
    await gm.getByTestId("combat-tracker:add-selected").click();

    await gm.getByRole("button", { name: "combatTracker.addEvent" }).click();
    await gm.getByLabel("combatTracker.eventName").fill("Trap trigger");
    await gm.getByLabel("combatTracker.eventLifespan").fill("1");
    await gm.getByLabel("combatTracker.eventMessage").fill("The floor gives way!");
    await gm.getByTestId("combat-tracker:add-event").click();

    await openTracker(player);

    // Roll all with 1d20 — both actor rows gain an initiative value, and the two rolls
    // (the event never rolls) each post a roll card to chat — combat intents reach chat, not
    // just the tracker's own state.
    await gm.getByLabel("combatTracker.notation").fill("1d20");
    await gm.getByTestId("combat-tracker:roll-all").click();
    await expect(gm.getByLabel("combatTracker.initiative").first()).not.toHaveValue("", { timeout: 15_000 });
    await expect(gm.locator(".roll-block")).toHaveCount(2, { timeout: 15_000 });
    await expect(player.locator(".roll-block")).toHaveCount(2, { timeout: 15_000 });

    // Start the combat — round 1, the first row is aria-current on BOTH browsers.
    await gm.getByRole("button", { name: "combatTracker.start" }).click();
    await expect(gm.getByText("combatTracker.round").first()).toBeVisible({ timeout: 15_000 });
    await expect(gm.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });
    await expect(player.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });

    // Whichever row is current, advance the clock until it lands on the player's own row so the
    // "your turn" notice and End-my-turn assertions below are deterministic regardless of turn
    // order (initiative is random). GM-side Advance covers every non-owner turn.
    for (let i = 0; i < 4; i++) {
      const playerRowCurrent = await player.locator('[aria-current="true"]').getAttribute("data-testid");
      if (playerRowCurrent) break;
      await gm.getByTestId("combat-tracker:advance").click();
    }

    await expect(player.getByText("combatTracker.yourTurn")).toBeVisible({ timeout: 15_000 });
    await player.getByTestId("combat-tracker:end-my-turn").click();
    await expect(gm.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });

    // Advance through the event's own turn: its lifespan (1) exhausts and the row disappears —
    // asserted on both browsers.
    for (let i = 0; i < 4; i++) {
      const eventGone = (await gm.getByText("Trap trigger").count()) === 0;
      if (eventGone) break;
      await gm.getByTestId("combat-tracker:advance").click();
    }
    await expect(gm.getByText("Trap trigger")).toHaveCount(0, { timeout: 15_000 });
    await expect(player.getByText("Trap trigger")).toHaveCount(0, { timeout: 15_000 });

    // The event's own turn posted its authored message to chat (a public "combat" channel
    // notice, `resolve_event`'s own message doc) — visible to both browsers.
    await expect(gm.getByText("The floor gives way!")).toBeVisible({ timeout: 15_000 });
    await expect(player.getByText("The floor gives way!")).toBeVisible({ timeout: 15_000 });

    // Hide the NPC (unassigned) row — it vanishes from the player's tracker live; reveal returns.
    const npcHideButton = gm.locator('[data-testid^="combat-tracker:hide-"]').last();
    const npcRowTestId = await npcHideButton.evaluate((el) =>
      el.closest('[data-testid^="combat-tracker:row-"]')?.getAttribute("data-testid"),
    );
    expect(npcRowTestId).toBeTruthy();
    await npcHideButton.click();
    await expect(player.getByTestId(npcRowTestId!)).toHaveCount(0, { timeout: 15_000 });
    await gm.locator('[data-testid^="combat-tracker:hide-"]').last().click();
    await expect(player.getByTestId(npcRowTestId!)).toHaveCount(1, { timeout: 15_000 });

    // Rewind — the previous row is current again on both browsers.
    await gm.getByTestId("combat-tracker:rewind").click();
    await expect(gm.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });
    await expect(player.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });

    // End — a two-click confirm — the tracker shows the empty state and the combat is gone for
    // the player.
    await gm.getByTestId("combat-tracker:end").click();
    await gm.getByTestId("combat-tracker:end").click();
    await expect(gm.getByText("combatTracker.noCombat")).toBeVisible({ timeout: 15_000 });
    await expect(player.getByText("combatTracker.noCombatPlayer")).toBeVisible({ timeout: 15_000 });

    // Compact viewport smoke, reusing the player context: the panel reflows into its compact
    // layout (this end-to-end assertion is a class check only). The ≥44px coarse-pointer touch
    // floor on every compact-branch button/input is pinned at the unit level, against each
    // component's own styles: `CombatTrackerPanel.touch.test.ts`'s "every button/input carries
    // the 44px coarse-target rule" case, covering CombatTrackerPanel/CombatHeader/CombatantRow.
    await player.setViewportSize({ width: 390, height: 844 });
    await expect(player.locator(".combat-tracker")).toHaveClass(/\bcompact\b/, { timeout: 15_000 });

    await closeTracker(player);
  } finally {
    await playerCtx.close();
  }
});
