import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page, Locator } from "@playwright/test";

// A member seated while the GM is connected appears in the GM's roster with no reload: the
// player redeems an invite, and the GM's already-open session adds the badge from the server's
// `member_profile` frame.

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** The roster badge whose accessible name starts with `name`. */
function badge(page: Page, name: string): Locator {
  return page.getByTestId("presence").locator(`[aria-label^="${name}"]`);
}

test("a member who redeems an invite appears in a connected GM's roster without a reload", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `seated-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Seating World ${Date.now().toString(36)}`;

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

  // Not seated yet: the GM's roster has no badge for the player.
  await expect(badge(gm, playerName)).toHaveCount(0);

  const playerCtx = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", {
      timeout: 30_000,
    });

    // The GM never reloaded: the badge arrives from the live frame, then goes online when the
    // player's own connection opens.
    await expect(badge(gm, playerName)).toHaveCount(1, { timeout: 15_000 });
    await expect(badge(gm, playerName)).toHaveAttribute("aria-label", /\(online\)$/, {
      timeout: 15_000,
    });
  } finally {
    await playerCtx.close();
  }
});
