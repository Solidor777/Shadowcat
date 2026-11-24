import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
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

/** Locates every `table_draw` chat card on `page` carrying either row label — the same `.card`
 * root `chat-media.spec.ts` locates by, filtered on the two labels this scenario's rows use.
 * @param page The page to search.
 * @returns A locator matching every such card.
 */
function drawCards(page: Page): Locator {
  return page.locator(".card").filter({ hasText: /Potion|Sword/ });
}

/** Reads a folder row's document id from its test id.
 * @param scope The panel to search.
 * @param name The folder's display name.
 * @returns The folder's document id.
 */
async function folderId(scope: Locator, name: string): Promise<string> {
  const node = scope.locator('[data-testid^="folder-node-"]:not([data-testid="folder-node-all"])', { hasText: name });
  const testId = await node.getAttribute("data-testid");
  return (testId ?? "").slice("folder-node-".length);
}

/** Drags `source`'s center onto `target`'s center with real pointer input.
 * @param page The page to drive.
 * @param source The drag handle.
 * @param target The drop target.
 */
async function dragOnto(page: Page, source: Locator, target: Locator): Promise<void> {
  const from = await source.boundingBox();
  const to = await target.boundingBox();
  if (!from || !to) throw new Error("drag endpoints are not laid out");
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 8 });
  await page.mouse.up();
}

test("tables: folders file tables, drag moves between folders, and deleting a folder keeps its tables", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, `Table Folders ${Date.now().toString(36)}`, account);
  await openPanel(page, "tables:panel");
  // The asset browser is open in a fresh world and carries the same folder test ids, so every
  // locator below is scoped to the tables panel.
  const panel = page.getByTestId("tables-panel");

  // Create a root folder.
  const folderName = panel.getByTestId("folder-create-name");
  await folderName.fill("Prep");
  await folderName.press("Enter");
  await expect(panel.locator('[data-testid^="folder-node-"]', { hasText: "Prep" })).toBeVisible();
  const prep = await folderId(panel, "Prep");

  // A table created at the root, and one created while the folder is selected.
  await panel.getByTestId("tables-name").fill("Alpha");
  await panel.getByTestId("tables-create").click();
  await panel.getByTestId(`folder-node-${prep}`).click();
  await panel.getByTestId("tables-name").fill("Beta");
  await panel.getByTestId("tables-create").click();
  await expect(panel.getByTestId("table-row")).toHaveCount(1);
  await expect(panel.getByTestId("table-row")).toContainText("Beta");

  // Under All both tables list; dragging Alpha's handle onto the folder files it there.
  await panel.getByTestId("folder-node-all").click();
  await expect(panel.getByTestId("table-row")).toHaveCount(2);
  const alpha = panel.getByTestId("table-row").filter({ hasText: "Alpha" });
  await dragOnto(page, alpha.getByTestId("table-drag"), panel.getByTestId(`folder-node-${prep}`));
  await panel.getByTestId(`folder-node-${prep}`).click();
  await expect(panel.getByTestId("table-row")).toHaveCount(2);

  // Dragging a table onto the All row takes it back to the root.
  await dragOnto(
    page,
    panel.getByTestId("table-row").filter({ hasText: "Alpha" }).getByTestId("table-drag"),
    panel.getByTestId("folder-node-all"),
  );
  await expect(panel.getByTestId("table-row")).toHaveCount(1);

  // Deleting the folder keeps its remaining table: it moves to the root.
  await panel.getByTestId(`folder-delete-${prep}`).click();
  await panel.getByTestId("folder-delete-confirm").click();
  await expect(panel.getByTestId(`folder-node-${prep}`)).toHaveCount(0);
  await panel.getByTestId("folder-node-all").click();
  await expect(panel.getByTestId("table-row")).toHaveCount(2);
  await expect(panel.getByTestId("table-row").filter({ hasText: "Beta" })).toHaveCount(1);
});

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

  const playerCtx = await newE2EContext(browser, { baseURL: test.info().project.use.baseURL });
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
    // call.

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
    // is already open from the earlier `openPanel` call above.
    await gm.getByTestId("table-quick-draw").click();
    await expect(drawCards(gm)).toHaveCount(2, { timeout: 15_000 });
    await expect(drawCards(player)).toHaveCount(2, { timeout: 15_000 });
  } finally {
    await playerCtx.close();
  }
});
