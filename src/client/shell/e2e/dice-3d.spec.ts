import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page } from "@playwright/test";

/** The 3D dice overlay host element on one page (`DiceOverlay`'s root). */
function dice3dOverlay(page: Page) {
  return page.locator(".dice3d-overlay");
}

test("a roll tumbles and settles to the server's result on both the GM and player screens", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Dice 3D World ${Date.now().toString(36)}`;

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

  const playerCtx = await browser.newContext({ baseURL: test.info().project.use.baseURL });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(player.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(gm.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // Chat panel docks by default on both sessions; send the roll from the GM.
    await gm.getByRole("textbox").fill("/roll 1d20");
    await gm.getByRole("textbox").press("Enter");

    const card = gm.locator(".card").filter({ has: gm.locator(".roll-total") });
    await expect(card).toHaveCount(1, { timeout: 15_000 });
    const total = await card.locator(".roll-total").textContent();
    expect(total).not.toBeNull();

    await expect(dice3dOverlay(gm)).toHaveAttribute("data-dice3d-state", "settled", { timeout: 8_000 });
    await expect(dice3dOverlay(player)).toHaveAttribute("data-dice3d-state", "settled", { timeout: 8_000 });
    const gmValues = await dice3dOverlay(gm).getAttribute("data-dice3d-values");
    const playerValues = await dice3dOverlay(player).getAttribute("data-dice3d-values");
    expect(gmValues).toBe(total!.trim());
    expect(playerValues).toBe(total!.trim());

    // The player turns dice3d off through the real Settings > Performance toggle; the next
    // roll leaves the player's overlay idle.
    await player.getByTestId("launcher-trigger").click();
    await player.getByTestId("launcher-item-settings:panel").click();
    await player.getByTestId("perf-dice3d").uncheck();

    await gm.getByRole("textbox").fill("/roll 1d6");
    await gm.getByRole("textbox").press("Enter");
    await expect(gm.locator(".card").filter({ has: gm.locator(".roll-total") })).toHaveCount(2, { timeout: 15_000 });
    await expect(dice3dOverlay(player)).toHaveAttribute("data-dice3d-state", "idle", { timeout: 5_000 });
  } finally {
    await playerCtx.close();
  }
});
