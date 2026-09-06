import { test, expect, login, createAccount } from "./fixtures";
import type { Page } from "@playwright/test";

const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

// Scene-space geometry: the camera starts at its default offset (0,0) scale 1, so a scene
// coordinate is exactly a canvas-local pixel. This scene is left at the default square grid
// (size 100) — the movement-budget gate is grid-shape-agnostic (it counts cells, not geometry),
// so a square grid keeps the arithmetic below simple and independent of any hex axial math.
// The token rides a cell-CENTER row: a start on a cell boundary leaves the router's start cell
// ambiguous, and with it the cell a truncated route lands on.
const TOKEN_Y = 350;
const PLACE_X = 200;
const HANDOFF_X = 250;
/** Three cells past the handoff point — a budget of 2 cells covers the first two, truncating the
 * third under `hard` enforcement. */
const TARGET_X = HANDOFF_X + 300;
/** The cell the truncated move must land on: exactly `movement`'s budget (2 cells × grid size
 * 100) past the handoff's own cell start. */
const TRUNCATED_X = HANDOFF_X + 200;

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

/** Turns grid snapping off — a scene-document write (`/engine/snapToGrid`), so it governs every
 * client's placements and drags, not just this page's. The pixel arithmetic above assumes it:
 * with snapping on, a placement or a 50px nudge lands on a cell CENTER instead.
 * @param page The GM's page (the toggle is GM-only).
 */
async function disableSnap(page: Page): Promise<void> {
  const snap = page.getByTestId("snap-toggle");
  await expect(snap).toHaveAttribute("aria-pressed", "true");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");
}

async function openGameSettings(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-game-settings:panel").click();
}

async function closeGameSettings(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-game-settings:panel").click();
}

async function dragScene(
  page: Page,
  from: { x: number; y: number },
  to: { x: number; y: number },
): Promise<void> {
  const box = await page.getByTestId("stage-canvas").boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + from.x, box!.y + from.y);
  await page.mouse.down();
  await page.mouse.move(box!.x + to.x, box!.y + to.y);
  await page.mouse.up();
}

/** Activates a tool rail tool unless it is already the active one — `ToolController.toggle`
 * turns a tool OFF on a second click of that same tool, so an unconditional click would
 * deactivate it.
 * @param page The page whose tool rail to drive.
 * @param id The tool id (`tool-<id>` test id).
 */
async function activateTool(page: Page, id: "select" | "measure"): Promise<void> {
  const button = page.getByTestId("tool-" + id);
  if ((await button.getAttribute("aria-pressed")) !== "true") await button.click();
  await expect(button).toHaveAttribute("aria-pressed", "true");
}

/** The single token's committed center, parsed from `data-token-positions` (`id:x,y`).
 * @param page The page whose stage to read.
 * @returns The token's center in scene units (canvas-local pixels at the default camera).
 */
async function tokenCenter(page: Page): Promise<{ x: number; y: number }> {
  const positions = (await stageHost(page).getAttribute("data-token-positions")) ?? "";
  const m = /:(-?[\d.]+),(-?[\d.]+)/.exec(positions);
  expect(m, `one token position in ${JSON.stringify(positions)}`).not.toBeNull();
  return { x: Number(m![1]), y: Number(m![2]) };
}

/** Route mode — the ONE interaction that produces a budget-labelled route preview: with the
 * token selected (the select tool's click), the measure tool's press-and-hover requests the
 * server's route for that token and draws its cost label with the Warn overage or Hard stop
 * suffix, which `Stage` mirrors onto the host as `data-measure-label` (the label otherwise
 * exists only as canvas-drawn content with no other DOM presence); the double-click at the goal
 * then commits the route through `moveRequest`. The select tool's own drag is feedback-only and
 * never labels. This is the one end-to-end proof that the resolved combat movement enforcement,
 * read from the REAL scene/world-settings documents through the real server round trip,
 * reaches this label — the label's own text/color logic is otherwise covered only at the unit
 * level (`measure-tool.test.ts`/`ToolRail.test.ts`). The hover is re-nudged on every poll so
 * a preview requested before the player's client held the combat's current rules is refreshed
 * rather than frozen (the preview fires per pointer move, leading-edge).
 * @param page The routing player's page.
 * @param from The token's center (the press point), in stage-canvas-local pixels.
 * @param to The route's goal, in stage-canvas-local pixels.
 * @param labelSubstring The substring the route-preview label must contain before the commit.
 */
async function routeExpectingLabel(
  page: Page,
  from: { x: number; y: number },
  to: { x: number; y: number },
  labelSubstring: string,
): Promise<void> {
  const box = await page.getByTestId("stage-canvas").boundingBox();
  expect(box).not.toBeNull();
  await activateTool(page, "select");
  await page.mouse.click(box!.x + from.x, box!.y + from.y);
  await activateTool(page, "measure");
  await page.mouse.move(box!.x + from.x, box!.y + from.y);
  await page.mouse.down();
  let nudge = 0;
  await expect
    .poll(async () => {
      nudge = (nudge + 1) % 2;
      await page.mouse.move(box!.x + to.x + nudge, box!.y + to.y);
      return (await stageHost(page).getAttribute("data-measure-label")) ?? "";
    }, { timeout: 15_000 })
    .toContain(labelSubstring);
  await page.mouse.up();
  await page.mouse.dblclick(box!.x + to.x, box!.y + to.y);
  await activateTool(page, "select");
}

test("the resource registry and combat chain editors drive a real movement-budget gate", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(180_000);
  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Combat Settings World ${Date.now().toString(36)}`;

  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();
  await createAccount(gm, playerName, playerPassword);
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();

  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // "Leave world" lives inside the settings panel's content, so that panel stays open until
    // this click; the persisted layout re-docks it on re-entry, and it closes only then.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-settings:panel").click();
    await disableSnap(gm);

    // --- Seat the player's actor first: image art, a named `system.speed` leaf authored
    // through the actor sheet's `SystemTreeEditor`, then a LINKED token stamped from the
    // selected actor. `gameSettings.resources.max`/`turnStart` below reference it by name
    // ("speed") rather than a numeric literal, exercising the real movement-budget gate
    // (Resource -> CombatDefaults -> execute_move) through `crate::formula`'s
    // `SystemLeafResolver`, not a stand-in.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();

    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    const actorsPanel = gm.locator(".actors");
    // Linked placement (the "independent copy" toggle off) so the token resolves `speed`
    // through the actor it links to, not an embedded copy.
    await actorsPanel.getByLabel("New independent copy on each placement").uncheck();
    await actorsPanel.getByPlaceholder("Name", { exact: true }).fill("PlayerChar");
    await actorsPanel.getByTestId("visual-pick").click();
    const pickDialog = gm.getByTestId("asset-pick-dialog");
    await expect(pickDialog).toBeVisible();
    await pickDialog.getByTestId("asset-tile").first().click();
    await pickDialog.getByTestId("pick-confirm").click();
    await expect(pickDialog).toHaveCount(0);
    await actorsPanel.getByRole("button", { name: "Create actor" }).click();

    await actorsPanel.getByRole("button", { name: "Open sheet" }).click();
    // `exact`: the floating panel wrapping the sheet is itself a dialog named "Sheet — …".
    const sheet = gm.getByRole("dialog", { name: "Sheet", exact: true });
    await expect(sheet).toBeVisible();
    await sheet.getByText("All data").click();
    await sheet.getByLabel("New field key").fill("speed");
    await sheet.getByLabel("New field value").fill("2");
    await sheet.getByRole("button", { name: "Add field" }).click();
    await expect(sheet.getByLabel("speed", { exact: true })).toHaveValue("2");
    await sheet.getByRole("button", { name: "Close" }).click();

    await actorsPanel.getByRole("button", { name: "PlayerChar" }).click();
    await gm.getByTestId("tool-place").click();
    let box = await gm.getByTestId("stage-canvas").boundingBox();
    expect(box).not.toBeNull();
    await gm.mouse.click(box!.x + PLACE_X, box!.y + TOKEN_Y);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    await openGameSettings(gm);

    // The player's route must be pathable at all: the default `visible` movement restriction
    // confines a player's route to the cells their token can see, and this token carries no
    // vision source. The gate under proof is the movement BUDGET, so the world tier lifts the
    // restriction (`hex-movement.spec.ts` authors the same).
    await gm.getByLabel("Movement restriction", { exact: true }).selectOption("unrestricted");
    await expect(gm.getByLabel("Movement restriction", { exact: true })).toHaveValue("unrestricted");

    // --- Resources editor: add a Tracked "movement" resource. `max`/`turnStart` reference the
    // actor's own named `system.speed` leaf authored above.
    await gm.getByLabel("Key", { exact: true }).fill("movement");
    await gm.getByRole("button", { name: "Add resource" }).click();
    await gm.getByLabel("Kind for resource movement", { exact: true }).selectOption("tracked");
    await gm.getByLabel("Max for resource movement", { exact: true }).fill("speed");
    await gm.getByLabel("Turn start for resource movement", { exact: true }).fill("speed");

    // --- Chain editor (world tier): movementResource = movement, interpretation = per_cell,
    // enforcement = hard.
    await gm.getByLabel("Movement resource", { exact: true }).selectOption("movement");
    await expect(gm.getByTestId("provenance:combat.movementResource")).toHaveText("World setting");
    // `spaces`: the resource IS the cell count (`per_cell` would divide `speed` by the scene's
    // distance-per-cell), so `speed` = 2 is the 2-cell budget the geometry above assumes.
    await gm.getByLabel("Budget interpretation", { exact: true }).selectOption("spaces");
    await gm.getByLabel("Enforcement", { exact: true }).selectOption("hard");
    await expect(gm.getByTestId("provenance:combat.enforcement")).toHaveText("World setting");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"hard"');

    // --- Scene tier: enforcement = warn overrides the world's hard on the selected scene.
    await gm.getByLabel("Enforcement (override)", { exact: true }).selectOption("warn");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"warn"');
    await expect(gm.getByTestId("provenance:combat.scene.enforcement")).toHaveText("Scene override");

    // Reset the scene override — falls back to the world's hard.
    await gm.getByLabel("Enforcement (override)", { exact: true }).selectOption("__inherit");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"hard"');

    // World reset — falls all the way back to the engine default (none) — then re-arm hard for
    // the gate proof below.
    await gm.getByLabel("Enforcement", { exact: true }).selectOption("__inherit");
    await expect(gm.getByTestId("provenance:combat.enforcement")).toHaveText("Engine default");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"none"');
    await gm.getByLabel("Enforcement", { exact: true }).selectOption("hard");
    await closeGameSettings(gm);

    // --- Assign the seated token's ownership (the actor + linked token were placed above,
    // before the game-settings edits).
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    box = await gm.getByTestId("stage-canvas").boundingBox();
    await gm.mouse.click(box!.x + PLACE_X, box!.y + TOKEN_Y);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });
    // That click also left the token selected for the tracker's "add selected" below.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    // --- Combat: create, add the selected token, start.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();
    await gm.getByTestId("combat-tracker:create").click();
    await expect(gm.getByTestId("combat-tracker:add-selected")).toBeEnabled({ timeout: 15_000 });
    await gm.getByTestId("combat-tracker:add-selected").click();
    await gm.getByRole("button", { name: "Start", exact: true }).click();
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();

    // Happens-before barrier: the GM's own handoff nudge, observed by the player, precedes every
    // player drag below (`hex-movement.spec.ts`'s pattern — frames arrive in sequence order on
    // one socket).
    await dragScene(gm, { x: PLACE_X, y: TOKEN_Y }, { x: HANDOFF_X, y: TOKEN_Y });
    await player.getByTestId("tool-select").click();
    await expect
      .poll(async () => (await stageHost(player).getAttribute("data-token-positions")) ?? "", { timeout: 20_000 })
      .toContain(":" + HANDOFF_X + ",");

    // --- Gate proof, hard enforcement: a route spanning 3 cells against a 2-cell budget is cut
    // at the 2-cell mark. Under Hard the cut happens at the server's route preview (the same
    // `budget_gate_for_token` + `resolve_budget` gate the executor runs, applied one stage
    // earlier), so the route the commit hands the executor is already the clamped 2-cell one and
    // `Stage`'s `data-last-move-outcome` — the server's own move-resolution outcome — reads
    // `executed`, never `truncated`: the executor is never handed an over-budget path through
    // the UI. The proof of the cut is the landing cell and the spent budget below, checked
    // independently of the route-preview label `routeExpectingLabel` confirms before the commit.
    await routeExpectingLabel(player, { x: HANDOFF_X, y: TOKEN_Y }, { x: TARGET_X, y: TOKEN_Y }, "stops at budget");
    await expect(stageHost(player)).toHaveAttribute("data-last-move-outcome", "executed", { timeout: 20_000 });
    await expect
      .poll(async () => (await stageHost(gm).getAttribute("data-token-positions")) ?? "", { timeout: 20_000 })
      .toContain(":" + TRUNCATED_X + ",");

    // The tracker's row shows the decremented `current` for the movement resource after the
    // truncated move consumed its full budget — for the GM (who may edit the resource) that
    // value is the cell's number input, not text.
    const resourceInput = gm.locator('[data-testid="combat-tracker:resource-movement"] input');
    await expect(resourceInput).toHaveValue("0", { timeout: 15_000 });

    // --- Re-authoring to warn: the overage executes in full instead of truncating. A running
    // combat carries the movement rules it SNAPSHOTTED when it started (`transition::start`
    // re-resolves the chain only on a fresh start), so the re-authored enforcement reaches a NEW
    // combat: end this one, then create and start another over the same selected token — its
    // fresh combatant reads its budget as full (lazy-full), so no refill is needed.
    await openGameSettings(gm);
    await gm.getByLabel("Enforcement", { exact: true }).selectOption("warn");
    await expect(gm.getByTestId("gameSettings:combat-effective-combat.enforcement")).toHaveText('"warn"');
    await closeGameSettings(gm);
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();
    await gm.getByTestId("combat-tracker:end").click();
    await gm.getByTestId("combat-tracker:end").click();
    await expect(gm.getByText("No combat running on this scene.")).toBeVisible({ timeout: 15_000 });
    await gm.getByTestId("combat-tracker:create").click();
    await expect(gm.getByTestId("combat-tracker:add-selected")).toBeEnabled({ timeout: 15_000 });
    await gm.getByTestId("combat-tracker:add-selected").click();
    await gm.getByRole("button", { name: "Start", exact: true }).click();
    await expect(gm.getByText("Round 1", { exact: true })).toBeVisible({ timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-combat-tracker:panel").click();

    // Warn enforcement's route preview shows the overage instead of a hard stop, and the move
    // executes in full. The route starts from the token's ACTUAL center — the truncated move
    // above parked it on a cell center, not on the pixel row it was placed on.
    const start = await tokenCenter(player);
    await routeExpectingLabel(player, start, { x: start.x + 300, y: start.y }, "over budget");
    await expect(stageHost(player)).toHaveAttribute("data-last-move-outcome", "executed", { timeout: 20_000 });
    await expect
      .poll(async () => (await stageHost(gm).getAttribute("data-token-positions")) ?? "", { timeout: 20_000 })
      .toContain(":" + (start.x + 300) + ",");
  } finally {
    await playerCtx.close();
  }
});
