import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene, dragScene } from "./stage-gestures";
import type { ScenePoint as Point } from "./stage-gestures";

// Multi-level authoring, viewing and teleport e2e: a GM authors two floors on the world's
// single (default, unnamed) scene via LevelsEditor, places a player-owned token on the lower
// floor and an NPC on the upper floor, confirms each viewer's stage exposes only its own
// floor's tokens (data-level/data-token-count), then authors a Teleport region trigger on the
// lower floor and walks the player into it — the player's floor and position both jump to the
// trigger's authored destination.
//
// The Teleport trigger's destination scene picker is left at its default (untouched): a scene
// document's own `search_text` arm is `String::new()` (`shadowcat::data::engine::search_text`'s
// `"scene"` match arm) and the world's default scene is created unnamed (`buildSceneDoc`'s
// `name: null`), so `ctx.searchDocuments({docTypes:["scene"]}, ...)` can never surface it — no
// UI anywhere in this codebase can rename a scene either. `PortalTarget.scene: null` already
// means "the portal's own scene" (see its own doc comment), so leaving the destination-scene
// field untouched is the CORRECT same-scene authoring path, not a workaround.

const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Scene-space geometry (square grid, 100-unit cells, snap OFF). The player's token starts on
// the ground floor, well clear of the teleport region; the region sits between the start point
// and a point the player then walks INTO, crossing its "enter" trigger.
const PLAYER_START = { x: 210, y: 310 };
const REGION_A = { x: 350, y: 260 }; // region drag anchor
const REGION_B = { x: 450, y: 360 }; // region drag release — a 100x100 box
const WALK_TARGET = { x: 400, y: 310 }; // inside the region box
const NPC_START = { x: 700, y: 700 }; // upper floor, also the teleport's authored destination
const TELEPORT_ELEVATION = "15"; // inside the upper floor's [10,20) band

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** `data-token-positions` is `id:x,y` pairs, id-sorted and `;`-joined (the per-spec parsing
 * convention for this attribute — only the canvas GESTURE primitives are centralized in
 * `stage-gestures.ts`). */
function parsePositions(positions: string): Map<string, Point> {
  const out = new Map<string, Point>();
  for (const entry of positions.split(";").filter((s) => s.length > 0)) {
    const [id, xy] = entry.split(":");
    const [x, y] = xy.split(",").map(Number);
    out.set(id, { x, y });
  }
  return out;
}

/** The id of the token at (approximately) a scene position, or null. */
function tokenIdNear(positions: Map<string, Point>, p: Point): string | null {
  for (const [id, at] of positions) {
    if (Math.hypot(at.x - p.x, at.y - p.y) < 5) return id;
  }
  return null;
}

/** Turn grid snapping off so every authored point is exactly the clicked scene coordinate. */
async function disableSnap(page: Page): Promise<void> {
  const snap = page.getByTestId("snap-toggle");
  await expect(snap).toHaveAttribute("aria-pressed", "true");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");
}

/** Upload one image through the asset-browser panel (real upload pipeline), leaving the panel
 * open on the freshly-grown tile grid. A caller-chosen filename and expected post-upload tile
 * count supports uploading several distinct assets in sequence (e.g. the token art, and one
 * background per floor), each identifiable afterward by its own filename.
 * @param page The GM's page.
 * @param filename The upload's filename (also the tile's `title`, used later to pick it back
 * out of the asset-pick overlay unambiguously).
 * @param expectedCount The tile count the asset-browser grid must reach after this upload.
 */
async function uploadAsset(page: Page, filename: string, expectedCount: number): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();
  await page.getByTestId("asset-upload-input").setInputFiles({
    name: filename,
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("asset-tile")).toHaveCount(expectedCount);
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
}

/** Opens the scene browser, opens the single default scene's `LevelsEditor`, authors two floors
 * ("Ground" [0,10) with `ground-bg.png`, "Upper" [10,20) with `upper-bg.png`), and returns their
 * server-assigned ids. `LevelsEditor.addLevel` generates a `crypto.randomUUID()` id with no
 * authorable override, so the ids are read back off each row's own `data-level-id` attribute
 * rather than assumed.
 * @param gm The GM's page.
 * @returns The two floors' ids, in authoring order (ground, upper).
 */
async function authorTwoLevels(gm: Page): Promise<{ groundId: string; upperId: string }> {
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-scene-browser:panel").click();
  await gm.getByTestId("levels-toggle").first().click();
  const editor = gm.getByTestId("levels-editor");
  await expect(editor).toBeVisible();

  await editor.getByTestId("level-add").click();
  await editor.getByTestId("level-add").click();
  const rows = editor.getByTestId("level-row");
  await expect(rows).toHaveCount(2);
  const groundId = await rows.nth(0).getAttribute("data-level-id");
  const upperId = await rows.nth(1).getAttribute("data-level-id");
  expect(groundId, "the first level row must carry a generated id").not.toBeNull();
  expect(upperId, "the second level row must carry a generated id").not.toBeNull();

  const groundRow = rows.nth(0);
  await groundRow.getByTestId("level-name").fill("Ground");
  await groundRow.getByTestId("level-background").click();
  const groundDialog = gm.getByTestId("asset-pick-dialog");
  await expect(groundDialog).toBeVisible();
  await groundDialog.getByTitle("ground-bg.png").click();
  await groundDialog.getByTestId("pick-confirm").click();
  await expect(groundDialog).not.toBeVisible();

  const upperRow = rows.nth(1);
  await upperRow.getByTestId("level-name").fill("Upper");
  await upperRow.getByTestId("level-bottom").fill("10");
  await upperRow.getByTestId("level-top").fill("20");
  await upperRow.getByTestId("level-background").click();
  const upperDialog = gm.getByTestId("asset-pick-dialog");
  await expect(upperDialog).toBeVisible();
  await upperDialog.getByTitle("upper-bg.png").click();
  await upperDialog.getByTestId("pick-confirm").click();
  await expect(upperDialog).not.toBeVisible();

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-scene-browser:panel").click();

  return { groundId: groundId!, upperId: upperId! };
}

test("levels: two floors, per-floor token scoping, and a Teleport region trigger", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Levels World ${Date.now().toString(36)}`;

  // --- GM session ---
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

  // --- Player session: a second browser context ---
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

    // Member roster is a session-start snapshot; the GM re-enters so the player appears.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await disableSnap(gm);
    // The player's token is a raw asset placement (no linked/instanced actor), so it carries no
    // vision source; under the default fail-closed `Visible` movement restriction the player's
    // own visibility mask would be empty and every non-GM route request would refuse as
    // Unreachable regardless of the region below. Set the WORLD tier permissive, matching
    // `hex-movement.spec.ts`'s own setup for the same reason: exercising levels/teleport geometry
    // needs no vision/lighting authoring, so unrestricted movement is the correct setup here too.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();
    await gm.getByLabel("Movement restriction", { exact: true }).selectOption("unrestricted");
    await expect(gm.getByLabel("Movement restriction", { exact: true })).toHaveValue("unrestricted");
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();

    await uploadAsset(gm, "token.png", 1);
    await uploadAsset(gm, "ground-bg.png", 2);
    await uploadAsset(gm, "upper-bg.png", 3);

    const { groundId, upperId } = await authorTwoLevels(gm);

    // The GM's own viewed floor defaults to the FIRST authored level (Ground).
    await expect(gm.getByTestId(`level-${groundId}`)).toHaveAttribute("aria-pressed", "true");

    // --- Place the player's token on Ground: the place tool stamps the GM's currently-viewed
    // floor's bottom onto the new token's elevation. `AssetPicker` mounts as soon as the place
    // tool activates (not gated on a scene click), and `controller.selectedAsset` persists
    // across placements (a stamp-tool convention, mirrored by every other spec) — so pick the
    // asset, then place with exactly ONE `clickScene`, matching `stage.spec.ts`'s convention.
    // A priming click here would, on a LATER placement, fire against the still-selected asset
    // from the PRIOR placement and create a spurious duplicate token. ---
    await gm.getByTestId("tool-place").click();
    const tokenPick = gm.getByTestId("picker-asset").first();
    await expect(tokenPick).toBeVisible({ timeout: 10_000 });
    await tokenPick.click();
    await clickScene(gm, PLAYER_START);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

    // Hand the placed token to the player: select it, then set its owner through the actors
    // panel.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    await clickScene(gm, PLAYER_START);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });

    // --- Switch to Upper and place the NPC token there. ---
    await gm.getByTestId(`level-${upperId}`).click();
    await expect(gm.getByTestId(`level-${upperId}`)).toHaveAttribute("aria-pressed", "true");
    await gm.getByTestId("tool-place").click();
    const npcPick = gm.getByTestId("picker-asset").first();
    await expect(npcPick).toBeVisible({ timeout: 10_000 });
    await npcPick.click();
    await clickScene(gm, NPC_START);
    // The Upper floor now carries one token (the NPC); Ground still carries the player's.
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

    // --- The player's stage is scoped to their own token's resolved floor (Ground): one token,
    // data-level = Ground's id. ---
    await expect(stageHost(player)).toHaveAttribute("data-level", groundId, { timeout: 15_000 });
    await expect(stageHost(player)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

    // --- Back to Ground: the GM confirms the player's token is the one visible there. ---
    await gm.getByTestId(`level-${groundId}`).click();
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    let playerTokenId: string | null = null;
    await expect
      .poll(
        async () => {
          const positions = parsePositions((await stageHost(gm).getAttribute("data-token-positions")) ?? "");
          playerTokenId = tokenIdNear(positions, PLAYER_START);
          return playerTokenId;
        },
        { message: "the GM's stage must report the player's committed position on Ground", timeout: 15_000 },
      )
      .not.toBeNull();

    // --- Author a Teleport region trigger on Ground, covering a cell the player will walk
    // through: destination = NPC_START (Upper floor's own starting point), elevation inside
    // Upper's band. The destination scene field is left untouched (see the file header comment
    // for why that is the correct same-scene authoring path). ---
    await gm.getByTestId("tool-region").click();
    await gm.getByTestId("region-trigger-add").click();
    await gm.getByTestId("region-trigger-effect").selectOption("teleport");
    await gm.getByTestId("region-trigger-teleport-x").fill(String(NPC_START.x));
    await gm.getByTestId("region-trigger-teleport-y").fill(String(NPC_START.y));
    const teleportElevation = gm.getByTestId("region-trigger-teleport-elevation");
    await teleportElevation.fill(TELEPORT_ELEVATION);
    // The teleport editor's numeric inputs commit on `change` (fired on blur), not on every
    // keystroke — `fill()` only dispatches `input`. The x/y fields above each get blurred for
    // free by the next field stealing focus, but nothing focusable follows the elevation field:
    // `<canvas>` carries no `tabindex`, so a subsequent `page.mouse.down()` on the stage canvas
    // never steals DOM focus and never blurs it. Force the blur explicitly, or `target.elevation`
    // stays at its unfilled default (`null` — "leave the token's current elevation unchanged")
    // and the teleport lands the player on the right floor's x/y at the WRONG elevation.
    await teleportElevation.blur();
    await dragScene(gm, REGION_A, REGION_B);

    // --- The player walks from their start point into the region: the "enter" trigger fires,
    // teleporting them to NPC_START at TELEPORT_ELEVATION (Upper's floor). ---
    await player.getByTestId("tool-select").click();
    await dragScene(player, PLAYER_START, WALK_TARGET);

    await expect(stageHost(player)).toHaveAttribute("data-level", upperId, { timeout: 20_000 });
    await expect
      .poll(
        async () => {
          const positions = parsePositions((await stageHost(player).getAttribute("data-token-positions")) ?? "");
          const at = playerTokenId ? positions.get(playerTokenId) : undefined;
          return at ? Math.hypot(at.x - NPC_START.x, at.y - NPC_START.y) < 5 : false;
        },
        { message: "the player's token must land at the teleport's authored destination", timeout: 20_000 },
      )
      .toBe(true);

    // --- The GM, viewing Upper, now sees both the NPC and the teleported player token. ---
    await gm.getByTestId(`level-${upperId}`).click();
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });
  } finally {
    await playerCtx.close();
  }
});
