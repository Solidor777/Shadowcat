import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Browser, BrowserContext, Page } from "@playwright/test";

// The compact scene-tools strip on a 360 px phone: a non-wrapping, horizontally scrolling row, so
// every button keeps its touch size and its whole label, and the paused note keeps a readable
// width without changing the strip's height.

const PHONE = { width: 360, height: 740 };
const DESKTOP = { width: 1600, height: 1000 };
const PAUSE_MESSAGE = "Back in five minutes";

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

/** The smallest touch target side, in CSS pixels. */
const TOUCH_MIN = 44;

/**
 * Asserts every button of the compact tool strip keeps the touch size and shows its whole label:
 * a squeezed button clips its text, which shows as content wider than the button's own box.
 * @param player The phone-sized player page.
 */
async function expectToolButtonsFit(player: Page): Promise<void> {
  const buttons = await player.locator(".tool-rail > button.tool").evaluateAll((els) =>
    els.map((el) => {
      const box = el.getBoundingClientRect();
      return {
        label: el.textContent?.trim() ?? "",
        width: box.width,
        height: box.height,
        clipped: el.scrollWidth > el.clientWidth,
        lines: Math.round(box.height / parseFloat(getComputedStyle(el).lineHeight || "1")),
      };
    }),
  );
  expect(buttons.length).toBeGreaterThan(3);
  for (const b of buttons) {
    expect(b.clipped, `label "${b.label}" is cut off`).toBe(false);
    expect(b.width, `"${b.label}" width`).toBeGreaterThanOrEqual(TOUCH_MIN);
    expect(b.height, `"${b.label}" height`).toBeGreaterThanOrEqual(TOUCH_MIN);
  }
}

/**
 * Creates a world as the GM, invites a player, and seats the player on a 360 px phone.
 * @param browser The Playwright browser, for the player's context.
 * @param gm The GM's page.
 * @param account The GM's account.
 * @param account.username The GM's user name.
 * @param account.password The GM's password.
 * @param baseURL The test server's origin.
 * @returns The player's page and context.
 */
async function seatPhonePlayer(
  browser: Browser,
  gm: Page,
  account: { username: string; password: string },
  baseURL: string | undefined,
): Promise<{ player: Page; playerCtx: BrowserContext }> {
  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Pause Note World ${Date.now().toString(36)}`;
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

  const playerCtx = await newE2EContext(browser, { baseURL, viewport: PHONE });
  const player = await playerCtx.newPage();
  await login(player, playerName, playerPassword);
  await player.getByLabel("Invite code").fill(code);
  await player.getByRole("button", { name: "Join with an invite code" }).click();
  await expect(player.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
  return { player, playerCtx };
}

/**
 * Pauses the session as the GM.
 * @param gm The GM's page.
 */
async function pauseSession(gm: Page): Promise<void> {
  await gm.getByTestId("topbar-pause-toggle").click();
  await gm.getByTestId("prompt-dialog-input").fill(PAUSE_MESSAGE);
  await gm.getByTestId("prompt-dialog-confirm").click();
  await expect(gm.getByTestId("topbar-pause-toggle")).toHaveAttribute("aria-pressed", "true");
}

test("the paused note keeps a readable width in the compact tool strip on a 360 px phone", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const { player, playerCtx } = await seatPhonePlayer(browser, page, account, test.info().project.use.baseURL);
  try {
    const baseRail = await player.locator(".tool-rail").boundingBox();
    await pauseSession(page);

    const note = player.getByTestId("moves-paused-note");
    await expect(note).toBeVisible({ timeout: 15_000 });
    const noteBox = await note.boundingBox();
    const railBox = await player.locator(".tool-rail").boundingBox();
    expect(noteBox).not.toBeNull();
    // The note takes its content width up to its cap instead of a share of a squeezed row, and it
    // does not change the strip's height.
    expect(noteBox!.width).toBeGreaterThanOrEqual(200);
    expect(railBox!.height).toBe(baseRail!.height);
  } finally {
    await playerCtx.close();
  }
});

test("every tool button keeps its label and the touch size in the compact strip, paused or not", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const { player, playerCtx } = await seatPhonePlayer(browser, page, account, test.info().project.use.baseURL);
  try {
    await expectToolButtonsFit(player);
    await pauseSession(page);
    await expect(player.getByTestId("moves-paused-note")).toBeVisible({ timeout: 15_000 });
    await expectToolButtonsFit(player);
  } finally {
    await playerCtx.close();
  }
});
