import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { sceneOrigin } from "./stage-gestures";

// Shared live ruler, two sessions: a player drags the measure tool and the GM's stage receives
// the relayed ruler and the player's own stage records none. With the player's share toggle off (its default) nothing reaches the GM;
// with it on, the GM's stage records the player's ruler while the drag is held and its stop when
// the pointer is released. Stage exposes the last received ruler as
// `data-last-ruler="<from>:<active>:<point count>"`.

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

test("a player's shared ruler reaches the GM while held and ends on release; off by default", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Ruler World ${Date.now().toString(36)}`;

  // --- GM session ---
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

  // --- Player session: a second browser context ---
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

    await player.getByTestId("tool-measure").click();
    const toggle = player.getByTestId("share-ruler-toggle");
    await expect(toggle).toHaveAttribute("aria-pressed", "false");

    // Sharing OFF (the default): a held drag sends nothing to the GM.
    const from = { x: 200, y: 300 };
    const to = { x: 500, y: 350 };
    let o = await sceneOrigin(player, from);
    await player.mouse.down();
    await player.mouse.move(o.x + to.x, o.y + to.y, { steps: 4 });
    await player.waitForTimeout(800);
    await expect(stageHost(gm)).not.toHaveAttribute("data-last-ruler", /.+/);
    await player.mouse.up();

    // Sharing ON: the held drag reaches the GM as an active ruler of two points...
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-pressed", "true");
    o = await sceneOrigin(player, from);
    await player.mouse.down();
    await player.mouse.move(o.x + to.x, o.y + to.y, { steps: 4 });
    await expect(stageHost(gm)).toHaveAttribute("data-last-ruler", /^[^:]+:true:2$/, { timeout: 15_000 });
    // The server relays the frame to the sender's own connection too; the sending client drops
    // it (it draws its own local preview), so its stage never records a remote ruler.
    await player.waitForTimeout(500);
    await expect(stageHost(player)).not.toHaveAttribute("data-last-ruler", /.+/);

    // ...and releasing the pointer ends it on the GM's stage.
    await player.mouse.up();
    await expect(stageHost(gm)).toHaveAttribute("data-last-ruler", /^[^:]+:false:0$/, { timeout: 15_000 });
  } finally {
    await playerCtx.close();
  }
});
