import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page } from "@playwright/test";

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

/** Logs `page` in and creates+enters a fresh world named `name`.
 * @param page The page to drive.
 * @param name The world's display name.
 * @param account The worker's GM account.
 */
async function enterFreshWorld(
  page: Page,
  name: string,
  account: { username: string; password: string },
): Promise<void> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
}

/** Opens a launcher-closed panel by its contribution id.
 * @param page The page to drive.
 * @param contributionId The panel contribution's id (e.g. `"notes:panel"`).
 */
async function openPanel(page: Page, contributionId: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${contributionId}`).click();
}

// Dual-session (GM + invited player), the `combat-tracker.spec.ts`/`hex-movement.spec.ts`
// seating flow: the GM creates the world/player account/invite, the player redeems it, and the
// GM re-enters so the freshly seated player is visible for the rest of the scenario.
test("notes: create, edit, share, roll from a shared body, and a shared child note", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Notes World ${Date.now().toString(36)}`;

  const gm = page;
  await enterFreshWorld(gm, worldName, account);

  await openPanel(gm, "settings:panel");
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
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // Re-enter as the GM so the freshly seated player is visible to the rest of the scenario
    // (AppContext's member roster is a session-start snapshot).
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // The GM creates a private note and opens its sheet.
    await openPanel(gm, "notes:panel");
    await gm.getByTestId("notes-name").fill("Session 1");
    await gm.getByTestId("notes-create").click();
    const gmSheet = gm.getByRole("dialog", { name: "Sheet", exact: true }).filter({ hasText: "Session 1" });
    await expect(gmSheet).toBeVisible({ timeout: 15_000 });

    // Edit the body: bold markdown plus an inline roll button, then Save.
    await gmSheet.getByTestId("note-edit").click();
    await gmSheet.getByTestId("note-source").fill("**bold** [[roll:1d6|Luck]]");
    await gmSheet.getByTestId("note-save").click();
    const gmBody = gmSheet.getByTestId("note-body");
    await expect(gmBody.locator("strong")).toBeVisible({ timeout: 15_000 });
    await expect(gmBody.getByRole("button", { name: "Luck" })).toBeVisible();

    // The player's notes panel is empty — the note is private by default.
    await openPanel(player, "notes:panel");
    await expect(player.getByTestId("note-row")).toHaveCount(0);

    // The GM shares it (visibility: shared); the player's panel now lists it live.
    await gmSheet.getByTestId("note-visibility").selectOption("observer");
    await expect(player.getByTestId("note-row")).toHaveCount(1, { timeout: 15_000 });

    // Opening it shows the body with NO textarea/Edit control (the player is a reader, not the
    // author, and never granted `core:edit_permissions`/write on the source).
    await player.getByTestId("note-open").click();
    const playerSheet = player
      .getByRole("dialog", { name: "Sheet", exact: true })
      .filter({ hasText: "Session 1" });
    await expect(playerSheet).toBeVisible({ timeout: 15_000 });
    await expect(playerSheet.getByTestId("note-edit")).toHaveCount(0);
    await expect(playerSheet.getByTestId("note-source")).toHaveCount(0);
    await expect(playerSheet.getByTestId("note-body").locator("strong")).toBeVisible();

    // The player clicks the roll button; the chat panel shows a roll card.
    await playerSheet.getByRole("button", { name: "Luck" }).click();
    await openPanel(player, "chat:panel");
    await expect(player.locator(".roll-block")).toHaveCount(1, { timeout: 15_000 });

    // The GM creates a child note from the sheet — private by default, so it doesn't yet reach
    // the player's tree under "Session 1" (no toggle renders: this recipient's view of the
    // parent has no children).
    await gmSheet.getByTestId("note-new-child").click();
    const gmChildSheet = gm
      .getByRole("dialog", { name: "Sheet", exact: true })
      .filter({ hasText: "Untitled note" });
    await expect(gmChildSheet).toBeVisible({ timeout: 15_000 });
    await openPanel(player, "notes:panel");
    await expect(player.getByTestId("note-toggle")).toHaveCount(0);

    // Sharing the child surfaces it under its parent (its `parent_id` resolves in the player's
    // view, so it nests rather than promoting to root).
    await gmChildSheet.getByTestId("note-visibility").selectOption("observer");
    await openPanel(player, "notes:panel");
    await expect(player.getByTestId("note-toggle")).toHaveCount(1, { timeout: 15_000 });
    await player.getByTestId("note-toggle").click();
    await expect(player.getByTestId("note-row")).toHaveCount(2);
  } finally {
    await playerCtx.close();
  }
});
