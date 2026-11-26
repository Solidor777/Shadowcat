import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Custom vision-kind e2e: the installed fixture module `vision-kind-fixture` (see
// `e2e-support/fixture-modules`) registers the treatment `fixture-glow` under
// `VISION_TREATMENT_CONTRACT`. A GM enables it through the module manager, authors a vision mode
// whose render hint is `custom` with that treatment, and a player whose token carries the mode sees
// the module's resolver paint the lighting frame: a fill by default, an outline once the hint's
// params ask for one. Observation goes through the stage's read-only `data-light-treatment`
// attribute ("<washed cells>,<outlined cells>"), never WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// The default scene is pitch dark (environment intensity 0), so the only cells the player's token
// perceives are the ones its custom mode reaches; snap is off so the click lands exactly here.
const LURKER = { x: 210, y: 310 };

/** The fixture module's folder id, as the module manager labels its checkbox. */
const FIXTURE_MODULE = "vision-kind-fixture";

/** The treatment name the fixture module registers. */
const FIXTURE_TREATMENT = "fixture-glow";

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

test("a module-registered custom vision kind paints the player's lighting frame with its own treatment", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Custom Kind World ${Date.now().toString(36)}`;

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
    await togglePanel(gm, "actors:panel");

    // Control: before the module is enabled the game-settings editor offers no custom render hint.
    await togglePanel(gm, "game-settings:panel");
    await gm.getByLabel("Add vision mode").click();
    const hintSelect = gm.getByLabel("Render hint for vision mode custom-1");
    await expect(hintSelect).toBeVisible();
    await expect(hintSelect.locator("option[value='custom']")).toHaveCount(0);
    await togglePanel(gm, "game-settings:panel");

    // Enable the installed fixture module for this world through the module manager; the save
    // reconciles the running session, so its treatment registers without a reload.
    // The settings panel has been open since the invite step; it is opened here only if closed.
    await gm.getByTestId("launcher-trigger").click();
    const settingsItem = gm.getByTestId("launcher-item-settings:panel");
    if ((await settingsItem.getAttribute("aria-checked")) !== "true") await settingsItem.click();
    else await gm.keyboard.press("Escape");
    const manager = gm.locator(".module-manager");
    await expect(manager).toBeVisible();
    await manager.getByRole("checkbox", { name: FIXTURE_MODULE }).check();
    await manager.getByRole("button", { name: "Save" }).click();
    await togglePanel(gm, "settings:panel");

    // The registered treatment makes the custom option appear, preselected to its name.
    await togglePanel(gm, "game-settings:panel");
    await expect(hintSelect.locator("option[value='custom']")).toHaveCount(1);
    await hintSelect.selectOption("custom");
    await expect(gm.getByLabel("Render hint treatment for vision mode custom-1")).toHaveValue(
      FIXTURE_TREATMENT,
    );
    await togglePanel(gm, "game-settings:panel");

    // Assign the mode to the actor.
    await togglePanel(gm, "actors:panel");
    await row.getByTestId("vision-add").click();
    await row.getByTestId("vision-mode-0").selectOption("custom-1");
    await row.getByTestId("vision-range-0").fill("12");
    await row.getByTestId("vision-range-0").press("Tab");

    // The treatment's default branch is a fill: washed cells, no outlined ones.
    await expect
      .poll(async () => (await treatment(player))?.[0] ?? 0, {
        message: "the module's fill paint must reach the cells the mode perceives",
        timeout: 20_000,
      })
      .toBeGreaterThan(0);
    expect((await treatment(player))?.[1]).toBe(0);

    // Params the module's resolver reads switch it to its outline branch.
    await togglePanel(gm, "actors:panel");
    await togglePanel(gm, "game-settings:panel");
    const params = gm.getByLabel("Render hint parameters for vision mode custom-1");
    await params.fill('{"paint":"outline"}');
    await params.press("Tab");
    await expect
      .poll(async () => (await treatment(player))?.[1] ?? 0, {
        message: "the module's outline paint must reach the cells the mode perceives",
        timeout: 20_000,
      })
      .toBeGreaterThan(0);
    expect((await treatment(player))?.[0]).toBe(0);

    // A treatment name no module registered resolves to no paint at all.
    await gm.getByLabel("Render hint treatment for vision mode custom-1").fill("unregistered-kind");
    await gm.getByLabel("Render hint treatment for vision mode custom-1").press("Tab");
    await expect
      .poll(async () => treatment(player), {
        message: "an unregistered treatment name paints nothing",
        timeout: 20_000,
      })
      .toEqual([0, 0]);
  } finally {
    await playerCtx.close();
  }
});
