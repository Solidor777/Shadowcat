import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

/** The fixture module's folder id, as the module manager labels its checkbox. */
const FIXTURE_MODULE = "live-module-fixture";

/** The custom property the fixture module's stylesheet sets on the document root. */
const FIXTURE_PROPERTY = "--live-module-fixture";

/** Reads the fixture stylesheet's custom property from the page's document root.
 * @param page The page to read.
 * @returns The trimmed property value, `""` while the stylesheet is not applied.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const value = await fixtureProperty(page);
 * ```
 */
async function fixtureProperty(page: Page): Promise<string> {
  return page.evaluate(
    (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim(),
    FIXTURE_PROPERTY,
  );
}

// The GM changes the world's enabled modules and the seated player's client follows live: the
// server's `modules_changed` frame makes every connection reconcile its loaded modules, so the
// fixture module's stylesheet is injected into (and later removed from) the player's page with no
// navigation. A marker set on the player's `window` ahead of the toggle survives it, which a reload
// would clear.
test("live modules: enabling and disabling a module changes a seated player's client with no reload", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Live Modules World ${Date.now().toString(36)}`;

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

    // Control: nothing of the module is present, and a marker proves the page is never reloaded.
    await player.evaluate(() => {
      (window as unknown as { __liveModulesMarker: boolean }).__liveModulesMarker = true;
    });
    const link = player.locator(`link[data-shadowcat-module-style="${FIXTURE_MODULE}"]`);
    await expect(link).toHaveCount(0);
    expect(await fixtureProperty(player)).toBe("");

    // The GM enables the module: the player's stylesheet appears and applies.
    const manager = gm.locator(".module-manager");
    await expect(manager).toBeVisible();
    await manager.getByRole("checkbox", { name: FIXTURE_MODULE }).check();
    await manager.getByRole("button", { name: "Save" }).click();
    await expect(link).toHaveCount(1, { timeout: 20_000 });
    await expect.poll(() => fixtureProperty(player)).toBe("active");

    // The GM disables it again: the stylesheet is removed from the player's page.
    await manager.getByRole("checkbox", { name: FIXTURE_MODULE }).uncheck();
    await manager.getByRole("button", { name: "Save" }).click();
    await expect(link).toHaveCount(0, { timeout: 20_000 });
    await expect.poll(() => fixtureProperty(player)).toBe("");

    // No navigation happened throughout.
    expect(
      await player.evaluate(() => (window as unknown as { __liveModulesMarker?: boolean }).__liveModulesMarker),
    ).toBe(true);
  } finally {
    await playerCtx.close();
  }
});
