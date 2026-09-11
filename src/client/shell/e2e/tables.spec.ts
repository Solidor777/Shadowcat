import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page, Locator } from "@playwright/test";

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
 * @param contributionId The panel contribution's id (e.g. `"tables:panel"`).
 */
async function openPanel(page: Page, contributionId: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${contributionId}`).click();
}

/** Locates every `table_draw` chat card on `page` carrying either row label — the same `.card`
 * root `chat-media.spec.ts` locates by, filtered on the two labels this scenario's rows use.
 * @param page The page to search.
 * @returns A locator matching every such card.
 */
function drawCards(page: Page): Locator {
  return page.locator(".card").filter({ hasText: /Potion|Sword/ });
}

// Dual-session (GM + invited player) so BOTH recipients' chat cards are asserted — the server is
// the sole authority for what each side ever sees, same rationale `combat-tracker.spec.ts` states
// for its own dual-session assertions.
test("tables: create, add rows, draw, and the panel's quick-draw both post cards visible to a player", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Tables World ${Date.now().toString(36)}`;

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

    // Both sides watch chat for the rest of the scenario. Chat is `defaultPlacement: docked`
    // — the ONE panel every session starts with already open — so it needs no `openPanel`
    // call; `activate`'s `ctx.panels.toggle` would instead CLOSE it here.

    // The GM creates "Loot" (world-readable by default: `buildTableDoc`'s `permissions.default:
    // "observer"`) and its sheet opens.
    await openPanel(gm, "tables:panel");
    await gm.getByTestId("tables-name").fill("Loot");
    await gm.getByTestId("tables-create").click();
    const sheet = gm.getByRole("dialog", { name: "Sheet", exact: true }).filter({ hasText: "Loot" });
    await expect(sheet).toBeVisible({ timeout: 15_000 });

    // Two rows, through the row editor. Weighted draw needs no lo/hi; `row-label` commits on
    // `change` (blur), so each fill is followed by leaving the field.
    await sheet.getByTestId("table-add-row").click();
    await sheet.getByTestId("table-add-row").click();
    const labels = sheet.getByTestId("row-label");
    await expect(labels).toHaveCount(2);
    await labels.nth(0).fill("Potion");
    await labels.nth(0).press("Tab");
    await labels.nth(1).fill("Sword");
    await labels.nth(1).press("Tab");

    // Draw from the sheet — a card lands on BOTH the GM's and the player's chat panel, each
    // carrying one of the two labels and, on every recipient, the roll tooltip trigger
    // (`RollTooltip` under `.table-draw-header` is rendered unconditionally — `spec`/`raw` are
    // GM-only server-side and never reach this component — so there is no GM/player markup
    // difference to assert).
    await sheet.getByTestId("table-draw").click();
    await expect(drawCards(gm)).toHaveCount(1, { timeout: 15_000 });
    await expect(drawCards(player)).toHaveCount(1, { timeout: 15_000 });
    await expect(drawCards(gm).locator(".table-draw-header .roll-tooltip-trigger")).toBeVisible();
    await expect(drawCards(player).locator(".table-draw-header .roll-tooltip-trigger")).toBeVisible();
    await expect(drawCards(gm).locator(".table-draw-row-label")).toHaveText(/Potion|Sword/);
    await expect(drawCards(player).locator(".table-draw-row-label")).toHaveText(/Potion|Sword/);

    // The panel's own quick Draw posts a second card, also visible to both. `tables:panel`
    // is ALREADY open (from the earlier `openPanel` above, never closed since) — a second
    // `openPanel` here would TOGGLE it closed via `activate`'s `ctx.panels.toggle`, hiding
    // `table-quick-draw` and hanging the click below on actionability for the rest of the
    // test's budget.
    await gm.getByTestId("table-quick-draw").click();
    await expect(drawCards(gm)).toHaveCount(2, { timeout: 15_000 });
    await expect(drawCards(player)).toHaveCount(2, { timeout: 15_000 });
  } finally {
    await playerCtx.close();
  }
});
