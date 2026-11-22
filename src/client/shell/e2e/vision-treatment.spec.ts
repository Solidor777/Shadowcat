import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Vision-treatment e2e: a GM authors a custom vision mode through the game-settings editor and
// gives it a render hint (a color wash, then an outline, then none); a player whose token carries
// that mode sees the matching treatment reach the painted lighting frame. Authoring goes through
// the real UI; observation goes through the stage's read-only `data-light-treatment` attribute
// ("<washed cells>,<outlined cells>"), never WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// The default scene is pitch dark (environment intensity 0), so the only cells the player's token
// perceives are the ones its custom mode reaches; snap is off so the click lands exactly here.
const LURKER = { x: 210, y: 310 };

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** The `[washed, outlined]` cell counts on a page's stage, or null before the first frame. */
async function treatment(page: Page): Promise<[number, number] | null> {
  const raw = await stageHost(page).getAttribute("data-light-treatment");
  if (raw === null) return null;
  const [wash, outline] = raw.split(",").map(Number);
  return [wash, outline];
}

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("a custom vision mode's render hint reaches the player's lighting frame", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Treatment World ${Date.now().toString(36)}`;

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

    await togglePanel(gm, "asset-browser:panel");
    await gm.getByTestId("asset-upload-input").setInputFiles({
      name: "tok.png",
      mimeType: "image/png",
      buffer: PNG_1X1,
    });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await togglePanel(gm, "asset-browser:panel");

    // A linked, player-owned actor placed on the stage.
    await togglePanel(gm, "actors:panel");
    const actors = gm.locator(".actors");
    await actors.getByLabel("Name", { exact: true }).fill("Lurker");
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
    await row.getByRole("button", { name: "Lurker" }).click();
    await gm.getByTestId("tool-place").click();
    await clickScene(gm, LURKER);
    await expect(stageHost(player)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

    // Control: with no treated mode assigned, nothing carries a treatment.
    await expect
      .poll(async () => (await stageHost(player).getAttribute("data-light-treatment")) !== null, {
        message: "the player's stage must have painted its first lighting frame",
        timeout: 20_000,
      })
      .toBe(true);
    expect(await treatment(player)).toEqual([0, 0]);

    // Author a custom vision mode with a color-wash hint through the game-settings editor.
    await togglePanel(gm, "actors:panel");
    await togglePanel(gm, "game-settings:panel");
    await gm.getByLabel("Add vision mode").click();
    const hintSelect = gm.getByLabel("Render hint for vision mode custom-1");
    await hintSelect.selectOption("colorWash");
    await expect(gm.getByLabel("Render hint opacity for vision mode custom-1")).toBeVisible();
    await togglePanel(gm, "game-settings:panel");

    // Assign the mode to the actor.
    await togglePanel(gm, "actors:panel");
    await row.getByTestId("vision-add").click();
    await row.getByTestId("vision-mode-0").selectOption("custom-1");
    await row.getByTestId("vision-range-0").fill("12");
    await row.getByTestId("vision-range-0").press("Tab");

    await expect
      .poll(async () => (await treatment(player))?.[0] ?? 0, {
        message: "the color-wash hint must reach the cells the mode perceives",
        timeout: 20_000,
      })
      .toBeGreaterThan(0);
    expect((await treatment(player))?.[1]).toBe(0);

    // Switch the hint to an outline: the wash count falls to zero, the outline count rises.
    await togglePanel(gm, "actors:panel");
    await togglePanel(gm, "game-settings:panel");
    await gm.getByLabel("Render hint for vision mode custom-1").selectOption("outline");
    await expect
      .poll(async () => (await treatment(player))?.[1] ?? 0, {
        message: "the outline hint must reach the cells the mode perceives",
        timeout: 20_000,
      })
      .toBeGreaterThan(0);
    expect((await treatment(player))?.[0]).toBe(0);

    // Clearing the hint removes every treatment.
    await gm.getByLabel("Render hint for vision mode custom-1").selectOption("");
    await expect
      .poll(async () => treatment(player), {
        message: "no hint, no treatment",
        timeout: 20_000,
      })
      .toEqual([0, 0]);
  } finally {
    await playerCtx.close();
  }
});
