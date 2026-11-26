import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene, dragScene } from "./stage-gestures";

// A 1×1 PNG used as token art (same fixture the stage suite uses).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

/** How far each session pans the camera right and down so scene coordinates left of and above the
 * origin land inside the canvas: scene `(x, y)` is canvas-local `(x + PAN_X, y + PAN_Y)`. The pan
 * is one drag on empty canvas before any tool is chosen. */
const PAN_X = 300;
const PAN_Y = 100;
const PAN_FROM = { x: 400, y: 400 };

/** Square grid cell size (the scene's default; the test never overrides it). */
const CELL = 100;
/** The margin the GM authors, in cells. Three cells reach scene x = -300, so every position this
 * test uses (all within two cells of the origin) lies inside the margin. */
const MARGIN_CELLS = 3;
/** Where the GM places the token: left of the authored rectangle, inside the margin (scene x < 0). */
const PLACE = { x: -150, y: 150 };
/** The GM's ownership-handoff nudge, still inside the margin. A raw GM write, so it need not land
 * on a cell center. */
const HANDOFF = { x: -100, y: 150 };
/** The player's destination: inside the authored rectangle, so the route must cross its left edge
 * (scene x = 0) out of the margin. A non-GM move snaps to the cell center: column 2, row 1. */
const TARGET = { x: 250, y: 150 };
const TARGET_CENTER_X = 2 * CELL + CELL / 2;

const VIEWPORT = { width: 1600, height: 1000 };

test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** Converts a scene point to the canvas-local point the camera pan maps it to.
 * @param p - The scene point.
 * @returns The canvas-local point.
 * @example
 * ```
 * canvasOf({ x: -150, y: 150 }); // { x: 150, y: 250 }
 * ```
 */
function canvasOf(p: { x: number; y: number }): { x: number; y: number } {
  return { x: p.x + PAN_X, y: p.y + PAN_Y };
}

/** Pans the camera by `(PAN_X, PAN_Y)` with one drag on empty canvas; no tool may be active yet, or
 * the tool could claim the gesture instead of the camera.
 * @param page - The page whose stage to pan.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * await panCamera(page);
 * ```
 */
async function panCamera(page: Page): Promise<void> {
  await dragScene(page, PAN_FROM, { x: PAN_FROM.x + PAN_X, y: PAN_FROM.y + PAN_Y });
}

/** The single token's committed `x` from `data-token-positions` (`id:x,y` pairs, `;`-joined).
 * @param positions - The attribute value.
 * @returns The token's x in scene units, negative inside the margin.
 * @example
 * ```
 * parseX("tok:-150,150"); // -150
 * ```
 */
function parseX(positions: string): number {
  const entries = positions.split(";").filter((s) => s.length > 0);
  expect(entries, "exactly one token is expected on this scene").toHaveLength(1);
  return Number(entries[0].slice(entries[0].indexOf(":") + 1).split(",")[0]);
}

async function expectTokenX(page: Page, x: number, why: string): Promise<void> {
  await expect
    .poll(async () => parseX((await stageHost(page).getAttribute("data-token-positions")) ?? ""), {
      message: why,
      timeout: 20_000,
    })
    .toBeCloseTo(x, -1);
}

// A scene margin is real play space: a token can stand left of the authored rectangle, and a
// non-GM player's server-routed move can start there and cross into the rectangle. Without the
// margin the token's start cell lies outside the world extent the router, the navmesh and the
// vision bound all read, so the player's pathfind is refused and the token never moves — which
// makes the final assertion fail if `padding` is not applied end to end (control to the setting
// itself: the same drag on a scene with margin 0 cannot succeed).
test("a player's routed move can leave a token placed at negative coordinates inside the scene margin", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Padding World ${Date.now().toString(36)}`;

  // --- GM session: the worker's admin account, who is also the world's owner/GM. ---
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

  // --- Player session: a second browser context (separate cookie jar). ---
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

    // The member roster is a session-start snapshot, so the GM re-enters the world for the
    // freshly seated player to be assignable as a token owner.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await panCamera(gm);
    await panCamera(player);

    // --- GM authors the margin and an unrestricted scene through the real controls. ---
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();
    // Unrestricted removes the visibility mask, so the margin's reach is the only thing under test.
    await gm.getByLabel("Movement restriction", { exact: true }).selectOption("unrestricted");
    const margin = gm.getByLabel("Margin around the scene (cells)", { exact: true });
    await margin.fill(String(MARGIN_CELLS));
    await margin.press("Tab"); // the input commits on change, which a number field fires on blur
    await expect(margin).toHaveValue(String(MARGIN_CELLS));
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();

    // Snap off, so the GM's raw placement and nudge land at exactly the named scene coordinates.
    const snap = gm.getByTestId("snap-toggle");
    await expect(snap).toHaveAttribute("aria-pressed", "true");
    await snap.click();
    await expect(snap).toHaveAttribute("aria-pressed", "false");

    // Token art, then the token itself: placed LEFT of the authored rectangle (scene x < 0).
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await gm.getByTestId("tool-place").click();
    const pick = gm.getByTestId("picker-asset").first();
    await expect(pick).toBeVisible({ timeout: 10_000 });
    await pick.click();
    await clickScene(gm, canvasOf(PLACE));
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await expectTokenX(gm, PLACE.x, "the token must be placed at a negative scene x inside the margin");
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();

    // Hand the token to the player (the per-token owner control lives in the Actors panel).
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    await clickScene(gm, canvasOf(PLACE));
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    // Happens-before barrier: once the player renders this nudge it has applied the ownership
    // write that preceded it, so the player's drag below cannot race ahead of its authorization.
    await dragScene(gm, canvasOf(PLACE), canvasOf(HANDOFF));
    await expectTokenX(player, HANDOFF.x, "the player must observe the GM's ownership handoff");
    await expectTokenX(gm, HANDOFF.x, "the GM's own nudge must commit");

    // --- The player drags the token out of the margin, across the authored rectangle's left edge. ---
    await player.getByTestId("tool-select").click();
    await dragScene(player, canvasOf(HANDOFF), canvasOf(TARGET));
    await expectTokenX(gm, TARGET_CENTER_X, "the server must route a move that starts inside the margin");
    await expectTokenX(player, TARGET_CENTER_X, "the routed move must stand on the player's client");
  } finally {
    await playerCtx.close();
  }
});
