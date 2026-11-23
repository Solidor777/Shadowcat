import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page, Locator } from "@playwright/test";

// Live roster, two sessions. A player who joins after the GM is online sees the GM's dot as
// online straight away (the Welcome's online set), and a profile change the GM saves reaches
// the other member's roster badge without a reconnect.

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** The roster badge whose accessible name starts with `name`. */
function badge(page: Page, name: string): Locator {
  return page.getByTestId("presence").locator(`[aria-label^="${name}"]`);
}

test("a late joiner sees who is online, and a profile change reaches another member live", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `roster-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Roster World ${Date.now().toString(36)}`;

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

    // The GM was online before the player connected: the player's roster shows it online.
    await expect(badge(player, account.username)).toHaveAttribute("aria-label", /\(online\)$/, {
      timeout: 15_000,
    });

    // The GM saves a colour from the settings panel it already has open; the player's badge for
    // the GM takes it with no reconnect.
    await gm.getByLabel("Colour").fill("#336699");
    await gm.getByRole("button", { name: "Save profile" }).click();
    await expect(badge(player, account.username)).toHaveCSS("background-color", "rgb(51, 102, 153)", {
      timeout: 15_000,
    });
  } finally {
    await playerCtx.close();
  }
});
