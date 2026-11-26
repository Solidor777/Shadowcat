import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";
import { latchAttributeValues } from "./transient-latch";

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

  // `newE2EContext` (not a plain `browser.newContext`) pins this context to the same
  // full-fidelity performance mirror the default `page` fixture gets: "auto" resolves to the
  // "mobile" preset (dice3d disabled) on a low-core-count host, and CI runners report
  // `hardwareConcurrency <= 4`, so a raw context reads that preset and this whole spec never
  // sees a die tumble there.
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

    // Chat panel docks by default on both sessions; send the roll from the GM. The composer's
    // label text is dynamic ("Message #{name}"/"Message the GM"), so target its stable id.
    // `settled` lasts 2.5 s and then falls back to `idle` by itself, and the two overlays settle
    // independently: latch both states before rolling so neither assertion depends on where a poll lands.
    const gmStates = await latchAttributeValues(dice3dOverlay(gm), "data-dice3d-state");
    const playerStates = await latchAttributeValues(dice3dOverlay(player), "data-dice3d-state");
    await gm.locator("#chat-composer-input").fill("/roll 1d20");
    await gm.locator("#chat-composer-input").press("Enter");

    const card = gm.locator(".card").filter({ has: gm.locator(".roll-total") });
    await expect(card).toHaveCount(1, { timeout: 15_000 });
    const total = await card.locator(".roll-total").textContent();
    expect(total).not.toBeNull();

    await expect.poll(() => gmStates.seen(), { timeout: 8_000 }).toContain("settled");
    await expect.poll(() => playerStates.seen(), { timeout: 8_000 }).toContain("settled");
    // `data-dice3d-values` is the last settled roll and persists after the overlay returns to idle.
    const gmValues = await dice3dOverlay(gm).getAttribute("data-dice3d-values");
    const playerValues = await dice3dOverlay(player).getAttribute("data-dice3d-values");
    expect(gmValues).toBe(total!.trim());
    expect(playerValues).toBe(total!.trim());

    // The player turns dice3d off through the real Settings > Performance toggle; the next
    // roll must never move the player's overlay out of idle.
    await player.getByTestId("launcher-trigger").click();
    await player.getByTestId("launcher-item-settings:panel").click();
    await player.getByTestId("perf-dice3d").uncheck();
    // The first roll's `settled` state falls back to idle by itself; wait for that so the history
    // length below is measured from rest.
    await expect(dice3dOverlay(player)).toHaveAttribute("data-dice3d-state", "idle", { timeout: 10_000 });
    const gmBefore = (await gmStates.transitions()).length;
    const playerBefore = (await playerStates.transitions()).length;

    await gm.locator("#chat-composer-input").fill("/roll 1d6");
    await gm.locator("#chat-composer-input").press("Enter");
    await expect(gm.locator(".card").filter({ has: gm.locator(".roll-total") })).toHaveCount(2, { timeout: 15_000 });
    // Positive control: the GM's own overlay does react to the second roll, so the roll reached the
    // overlays and the interval below is long enough for a player overlay to have started.
    await expect.poll(async () => (await gmStates.transitions()).length, { timeout: 8_000 }).toBeGreaterThan(gmBefore);
    await player.waitForTimeout(1_500);
    // A read of the current state would say "idle" whether or not the overlay ran and finished; the
    // ordered history shows whether it ever left idle.
    expect((await playerStates.transitions()).length).toBe(playerBefore);
    await expect(dice3dOverlay(player)).toHaveAttribute("data-dice3d-state", "idle");
  } finally {
    await playerCtx.close();
  }
});
