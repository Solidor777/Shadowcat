import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene, dragScene } from "./stage-gestures";
import { latchTextSeen } from "./transient-latch";

// Whole-session pause, two sessions: the GM pauses with a message; the player sees the banner,
// the player's token drag is refused with the paused notice and the token stays put, chat still
// works, and after the GM resumes the same drag moves the token. The notice is a toast, so its
// latch is installed before the drag and read afterwards. Token positions are observed through
// the stage's read-only `data-token-positions` attribute, never WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Snap is off so the press lands exactly on the token and the move is not rounded away.
// Both points are canvas-local and inside the canvas, which the docked panels narrow below the
// viewport width.
const TOKEN_AT = { x: 300, y: 460 };
const TOKEN_TO = { x: 500, y: 460 };
const PAUSE_MESSAGE = "Back in five minutes";

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("a paused session freezes a player's token and combat controls, keeps chat, and resumes", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Pause World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await togglePanel(gm, "settings:panel");
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

    // The member roster is a session-start snapshot: re-enter so the player appears as an owner.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    const snap = gm.getByTestId("snap-toggle");
    await expect(snap).toHaveAttribute("aria-pressed", "true");
    await snap.click();
    await expect(snap).toHaveAttribute("aria-pressed", "false");

    // Unrestricted movement removes the visibility mask: the player's token carries no sense, so
    // under the default restriction no drag could ever move it and the resume step would prove nothing.
    await togglePanel(gm, "game-settings:panel");
    await gm.getByLabel("Movement restriction", { exact: true }).selectOption("unrestricted");
    await togglePanel(gm, "game-settings:panel");

    await togglePanel(gm, "asset-browser:panel");
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await togglePanel(gm, "asset-browser:panel");

    // A linked actor the player owns, placed on the scene.
    await togglePanel(gm, "actors:panel");
    const actors = gm.locator(".actors");
    await actors.getByLabel("Name", { exact: true }).fill("Mover");
    await actors.getByLabel("New independent copy on each placement").uncheck();
    await actors.getByTestId("visual-pick").click();
    const dialog = gm.getByTestId("asset-pick-dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByTestId("asset-tile").first().click();
    await dialog.getByTestId("pick-confirm").click();
    await expect(dialog).not.toBeVisible();
    await actors.getByRole("button", { name: "Create actor" }).click();
    const row = actors.getByRole("listitem");
    await expect(row).toHaveCount(1);
    await row.getByLabel("Owner").selectOption({ label: playerName });
    await row.getByRole("button", { name: "Mover" }).click();
    await gm.getByTestId("tool-place").click();
    await clickScene(gm, TOKEN_AT);
    await expect(stageHost(player)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await gm.getByTestId("tool-select").click();

    // The GM pauses with a message through the topbar toggle and its prompt.
    await gm.getByTestId("topbar-pause-toggle").click();
    await gm.getByTestId("prompt-dialog-input").fill(PAUSE_MESSAGE);
    await gm.getByTestId("prompt-dialog-confirm").click();
    await expect(gm.getByTestId("topbar-pause-toggle")).toHaveAttribute("aria-pressed", "true");

    // Every member sees the banner with the message; only the GM has the controls.
    await expect(player.getByTestId("session-pause-banner")).toContainText(PAUSE_MESSAGE, { timeout: 15_000 });
    await expect(gm.getByTestId("session-pause-banner")).toContainText(PAUSE_MESSAGE);
    await expect(player.getByTestId("topbar-pause-toggle")).toHaveCount(0);

    // The player's drag is refused: the paused notice shows and the token stays where it was.
    await player.getByTestId("tool-select").click();
    await expect(player.getByTestId("tool-select")).toHaveAttribute("data-moves-frozen", "true");
    const before = await stageHost(player).getAttribute("data-token-positions");
    expect(before).toBeTruthy();
    const pausedNotice = await latchTextSeen(player, "The game is paused.");
    await clickScene(player, TOKEN_AT);
    await dragScene(player, TOKEN_AT, TOKEN_TO, 4);
    await expect.poll(() => pausedNotice.seen(), { timeout: 15_000 }).toBe(true);
    await player.waitForTimeout(500);
    await expect(stageHost(player)).toHaveAttribute("data-token-positions", before!);

    // Chat is not part of the pause.
    const text = `still talking ${Date.now().toString(36)}`;
    await player.locator("#chat-composer-input").fill(text);
    await player.locator("#chat-composer-input").press("Enter");
    await expect(gm.getByText(text)).toBeVisible({ timeout: 15_000 });

    // The GM resumes; the banner clears and the same drag now moves the token.
    await gm.getByTestId("topbar-pause-toggle").click();
    await expect(gm.getByTestId("session-pause-banner")).toHaveCount(0);
    await expect(player.getByTestId("session-pause-banner")).toHaveCount(0, { timeout: 15_000 });
    await expect(player.getByTestId("tool-select")).not.toHaveAttribute("data-moves-frozen", "true");
    await clickScene(player, TOKEN_AT);
    await dragScene(player, TOKEN_AT, TOKEN_TO, 4);
    await expect(stageHost(player)).not.toHaveAttribute("data-token-positions", before!, { timeout: 15_000 });
  } finally {
    await playerCtx.close();
  }
});
