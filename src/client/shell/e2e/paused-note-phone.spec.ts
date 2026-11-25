import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";

// The paused note in the scene-tools rail on a 360 px phone: the compact strip is a non-wrapping
// row, so the note must keep a readable width and leave the strip's height alone.

const PHONE = { width: 360, height: 740 };
const DESKTOP = { width: 1600, height: 1000 };
const PAUSE_MESSAGE = "Back in five minutes";

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("the paused note keeps a readable width in the compact tool strip on a 360 px phone", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Pause Note World ${Date.now().toString(36)}`;

  const gm = page;
  await gm.setViewportSize(DESKTOP);
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(gm.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await togglePanel(gm, "settings:panel");
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await newE2EContext(browser, {
    baseURL: test.info().project.use.baseURL,
    viewport: PHONE,
  });
  const player = await playerCtx.newPage();
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(player.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    const baseRail = await player.locator(".tool-rail").boundingBox();
    await gm.getByTestId("topbar-pause-toggle").click();
    await gm.getByTestId("prompt-dialog-input").fill(PAUSE_MESSAGE);
    await gm.getByTestId("prompt-dialog-confirm").click();
    await expect(gm.getByTestId("topbar-pause-toggle")).toHaveAttribute("aria-pressed", "true");

    const note = player.getByTestId("moves-paused-note");
    await expect(note).toBeVisible({ timeout: 15_000 });
    const rail = player.locator(".tool-rail");
    const noteBox = await note.boundingBox();
    const railBox = await rail.boundingBox();
    expect(noteBox).not.toBeNull();
    // The note takes its content width up to its cap instead of a share of a squeezed row, and it
    // does not change the strip's height.
    expect(noteBox!.width).toBeGreaterThanOrEqual(200);
    expect(railBox!.height).toBe(baseRail!.height);
  } finally {
    await playerCtx.close();
  }
});
