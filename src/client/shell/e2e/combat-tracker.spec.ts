import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page } from "@playwright/test";
import { clickScene } from "./stage-gestures";
import type { ScenePoint } from "./stage-gestures";

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

/** Activates the place tool, picks the first asset once, then places one raw token per
 * point. The tool stays active and the picked asset persists across placements (a second
 * `tool-place` click would toggle the tool OFF), so both are done exactly once.
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
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Combat Tracker World ${Date.now().toString(36)}`;

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

    // Re-enter as the GM so the freshly seated player is assignable as a token owner. "Leave
    // world" lives inside the settings panel's content, so that panel stays open until this
    // click; the persisted layout re-docks it on re-entry, and it closes only then.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-settings:panel").click();

    // Token art, then two tokens: one for the player, one an unassigned NPC.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);

    await placeTokens(gm, [{ x: 200, y: 300 }, { x: 400, y: 300 }]);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();

    // Assign the first token's owner to the player via the actors panel.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    await clickScene(gm, { x: 200, y: 300 });
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });

    // Extend the selection to the NPC token for the tracker's "add selected": Shift+click adds
    // to the select tool's selection (a plain click replaces it; a pointer-down on empty
    // ground clears it — there is no marquee).
    await gm.keyboard.down("Shift");
    await clickScene(gm, { x: 400, y: 300 });
    await gm.keyboard.up("Shift");
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    // Open the tracker; create the combat and add both selected tokens plus an event.
    await openTracker(gm);
    await gm.getByTestId("combat-tracker:create").click();
    await expect(gm.getByTestId("combat-tracker:add-selected")).toBeEnabled({ timeout: 15_000 });
    await gm.getByTestId("combat-tracker:add-selected").click();

    await gm.getByRole("button", { name: "Add event" }).click();
    await gm.getByLabel("Event name", { exact: true }).fill("Trap trigger");
    await gm.getByLabel("Turns remaining", { exact: true }).fill("1");
    await gm.getByLabel("Message", { exact: true }).fill("The floor gives way!");
    await gm.getByTestId("combat-tracker:add-event").click();

    await openTracker(player);

    // Roll all with 1d20 — both actor rows gain an initiative value, and the two rolls
    // (the event never rolls) each post a roll card to chat — combat intents reach chat, not
    // just the tracker's own state.
    await gm.getByLabel("Notation", { exact: true }).fill("1d20");
    await gm.getByTestId("combat-tracker:roll-all").click();
    await expect(gm.getByLabel("Initiative", { exact: true }).first()).not.toHaveValue("", { timeout: 15_000 });
    await expect(gm.locator(".roll-block")).toHaveCount(2, { timeout: 15_000 });
    await expect(player.locator(".roll-block")).toHaveCount(2, { timeout: 15_000 });

    // The "your turn" notice is an auto-dismissing toast that fires the moment the clock lands on
    // the player's own row — which may be the very first turn — so the watch for it starts
    // BEFORE Start and is awaited once the advance loop below has landed there.
    const yourTurnNotice = expect(player.getByText("It's your turn!")).toBeVisible({ timeout: 60_000 });

    // Start the combat — round 1, the first row is aria-current on BOTH browsers.
    await gm.getByRole("button", { name: "Start", exact: true }).click();
    await expect(gm.getByText("Round 1", { exact: true })).toBeVisible({ timeout: 15_000 });
    await expect(gm.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });
    await expect(player.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });

    // Whichever row is current, advance the clock until it lands on the player's own row so the
    // "your turn" notice and End-my-turn assertions below are deterministic regardless of turn
    // order (initiative is random). "End my turn" renders on the player's header only during
    // their own turn (owner-may-end turn control), so it is the player-side signal; each GM-side
    // Advance is followed by waiting for the player's tracker to show the same current row, so
    // the check never runs against a stale view and skips past the player's turn.
    const gmCurrentRow = gm.locator('[aria-current="true"]');
    for (let i = 0; i < 4; i++) {
      if ((await player.getByTestId("combat-tracker:end-my-turn").count()) > 0) break;
      const before = await gmCurrentRow.getAttribute("data-testid");
      await gm.getByTestId("combat-tracker:advance").click();
      await expect(gmCurrentRow).not.toHaveAttribute("data-testid", before!, { timeout: 15_000 });
      const after = await gmCurrentRow.getAttribute("data-testid");
      await expect(player.locator('[aria-current="true"]')).toHaveAttribute("data-testid", after!, { timeout: 15_000 });
    }

    await yourTurnNotice;
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
    // The NPC row is the one the PLAYER cannot act on: their own row carries the editable
    // initiative input, the NPC's shows initiative as text.
    const npcRowTestId = await player
      .locator('[data-testid^="combat-tracker:row-"]')
      .filter({ hasNot: player.getByLabel("Initiative", { exact: true }) })
      .getAttribute("data-testid");
    expect(npcRowTestId).toBeTruthy();
    const npcHideButton = gm.getByTestId(npcRowTestId!.replace("combat-tracker:row-", "combat-tracker:hide-"));
    await npcHideButton.click();
    await expect(player.getByTestId(npcRowTestId!)).toHaveCount(0, { timeout: 15_000 });
    await npcHideButton.click();
    await expect(player.getByTestId(npcRowTestId!)).toHaveCount(1, { timeout: 15_000 });

    // Rewind — the previous row is current again on both browsers.
    await gm.getByTestId("combat-tracker:rewind").click();
    await expect(gm.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });
    await expect(player.locator('[aria-current="true"]')).toHaveCount(1, { timeout: 15_000 });

    // End — a two-click confirm — the tracker shows the empty state and the combat is gone for
    // the player.
    await gm.getByTestId("combat-tracker:end").click();
    await gm.getByTestId("combat-tracker:end").click();
    await expect(gm.getByText("No combat running on this scene.")).toBeVisible({ timeout: 15_000 });
    await expect(player.getByText("No combat is running.")).toBeVisible({ timeout: 15_000 });

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
