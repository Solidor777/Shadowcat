import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";
import { latchTextSeen } from "./transient-latch";

// Present note, two sessions: the GM presses "Present to players" on a PRIVATE note. The sheet
// first shares it (the server never presents an unshared note), then the presentation signal makes
// the player's client open the note's sheet panel on its own and raise the "presenting" toast. The
// toast is transient, so its latch is installed before the press.

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

test("a GM presenting a private note shares it and opens it on the player's screen", async ({ page, browser, account }) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Present World ${Date.now().toString(36)}`;
  const noteName = "Handout";

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

  const playerCtx = await newE2EContext(browser, {
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // The GM creates a private note with a body and opens its sheet.
    await openPanel(gm, "notes:panel");
    await gm.getByTestId("notes-name").fill(noteName);
    await gm.getByTestId("notes-create").click();
    const gmSheet = gm.getByRole("dialog", { name: "Sheet", exact: true }).filter({ hasText: noteName });
    await expect(gmSheet).toBeVisible({ timeout: 15_000 });
    await gmSheet.getByTestId("note-edit").click();
    await gmSheet.getByTestId("note-source").fill("**Read this aloud**");
    await gmSheet.getByTestId("note-save").click();
    await expect(gmSheet.getByTestId("note-body").locator("strong")).toBeVisible({ timeout: 15_000 });
    await expect(gmSheet.getByTestId("note-visibility")).toHaveValue("none");

    // Before presenting, the player has no sheet open and sees no note.
    await expect(player.getByRole("dialog", { name: "Sheet", exact: true })).toHaveCount(0);

    const presenting = await latchTextSeen(player, `The GM is presenting: ${noteName}`);
    await gmSheet.getByTestId("note-present").click();

    // The sheet shared the note, and the player's sheet opened by itself with the body visible.
    await expect(gmSheet.getByTestId("note-visibility")).toHaveValue("observer", { timeout: 15_000 });
    const playerSheet = player.getByRole("dialog", { name: "Sheet", exact: true }).filter({ hasText: noteName });
    await expect(playerSheet).toBeVisible({ timeout: 15_000 });
    await expect(playerSheet.getByTestId("note-body").locator("strong")).toHaveText("Read this aloud");
    await expect.poll(() => presenting.seen(), { timeout: 15_000 }).toBe(true);
  } finally {
    await playerCtx.close();
  }
});
