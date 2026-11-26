import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAACXBIWXMAAAABAAAAAQBPJcTWAAAAEElEQVR4nGP8ywACLGCSAQANEQED1LYyQAAAAABJRU5ErkJggg==",
  "base64",
);

// A player edits their own profile in the settings panel; the GM's roster badge for that player
// follows live, with no reload: the pronouns join the badge's title, an uploaded avatar replaces
// the initial glyph, and removing the avatar brings the glyph back. The server's `member_profile`
// frame carries each change to the other member.
test("profile: a player's pronouns and avatar show live on the GM's roster", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Profile World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await openPanel(gm, "settings:panel");
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await newE2EContext(browser, { baseURL: test.info().project.use.baseURL });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // The GM's roster lists the player by name until a profile says more.
    const badge = gm.getByTestId("presence").locator(`[title^="${playerName}"]`);
    await expect(badge).toHaveCount(1, { timeout: 15_000 });
    await expect(badge).toHaveAttribute("title", playerName);

    await openPanel(player, "settings:panel");
    const profile = player.locator(".profile");
    await expect(profile).toBeVisible();

    // Pronouns reach the GM's badge.
    await profile.getByLabel("Pronouns").fill("she/her");
    await profile.getByRole("button", { name: "Save profile" }).click();
    await expect(badge).toHaveAttribute("title", `${playerName} (she/her)`, { timeout: 15_000 });

    // A changed value replaces the old one.
    await profile.getByLabel("Pronouns").fill("they/them");
    await profile.getByRole("button", { name: "Save profile" }).click();
    await expect(badge).toHaveAttribute("title", `${playerName} (they/them)`, { timeout: 15_000 });

    // An uploaded avatar replaces the initial glyph on the GM's badge.
    await expect(badge.locator("img")).toHaveCount(0);
    await profile.getByLabel("Avatar").setInputFiles({ name: "me.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(badge.locator("img")).toHaveCount(1, { timeout: 15_000 });

    // Removing it brings the glyph back.
    await profile.getByRole("button", { name: "Remove avatar" }).click();
    await expect(badge.locator("img")).toHaveCount(0, { timeout: 15_000 });
    await expect(badge).toContainText(playerName.charAt(0).toUpperCase());
  } finally {
    await playerCtx.close();
  }
});
