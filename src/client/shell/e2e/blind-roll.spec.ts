import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";
import { latchAttributeValues } from "./transient-latch";

/** The 3D dice overlay host element on one page (`DiceOverlay`'s root). */
function dice3dOverlay(page: Page) {
  return page.locator(".dice3d-overlay");
}

test("a blind roll shows the roller only a 'rolled blind' card and the GM the full roll", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Blind Roll World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(gm.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  // `newE2EContext` pins the player's context to the full-fidelity performance mirror, so the
  // player's 3D dice overlay is enabled and its silence below is the feature's doing.
  const playerCtx = await newE2EContext(browser, { baseURL: test.info().project.use.baseURL });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(player.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(gm.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // Latch both overlays' states before the roll: the GM's must reach "settled", the player's
    // must never leave "idle".
    const gmStates = await latchAttributeValues(dice3dOverlay(gm), "data-dice3d-state");
    const playerStates = await latchAttributeValues(dice3dOverlay(player), "data-dice3d-state");
    const playerBefore = (await playerStates.transitions()).length;

    // The player picks the blind roll mode from the composer's roll-mode picker, then sends.
    await player.locator("#chat-composer-input").fill("1d20");
    await player.getByTestId("roll-mode").selectOption({ label: "Blind roll (only the GM sees it)" });
    await expect(player.locator("#chat-composer-input")).toHaveValue("/blindroll 1d20");
    await player.locator("#chat-composer-input").press("Enter");

    // GM: the full roll with a result and the blind badge, and the dice tumble.
    const gmCard = gm.locator(".card").filter({ has: gm.locator(".roll-total") });
    await expect(gmCard).toHaveCount(1, { timeout: 15_000 });
    await expect(gmCard.getByTestId("blind-badge")).toBeVisible();
    await expect(gmCard.locator(".roll-formula")).toHaveText("1d20");
    const total = (await gmCard.locator(".roll-total").textContent())?.trim();
    expect(total).toMatch(/^\d+$/);
    await expect.poll(() => gmStates.seen(), { timeout: 8_000 }).toContain("settled");
    expect(await dice3dOverlay(gm).getAttribute("data-dice3d-values")).toBe(total);

    // Player: only the compact card, no formula, no total, no tumble.
    const blindCard = player.getByTestId("blind-roll-card");
    await expect(blindCard).toHaveCount(1, { timeout: 15_000 });
    await expect(blindCard).toHaveText("Rolled blind");
    await expect(player.locator(".roll-total")).toHaveCount(0);
    await expect(player.locator(".roll-formula")).toHaveCount(0);
    await expect(player.getByTestId("blind-badge")).toHaveCount(0);
    // The GM's overlay left idle for this roll (positive control), so the interval is long
    // enough for the player's overlay to have started had it been going to.
    await player.waitForTimeout(1_500);
    expect((await playerStates.transitions()).length).toBe(playerBefore);
  } finally {
    await playerCtx.close();
  }
});
