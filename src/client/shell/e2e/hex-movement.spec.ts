import { test, expect, type Page, type Locator } from "@playwright/test";

// A 1×1 PNG used as token art (same fixture the stage suite uses).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Scene-space geometry for this spec. The camera starts at offset (0,0) scale 1
// (`render/src/camera.ts`) and nothing here pans or zooms, so a scene coordinate
// is exactly a canvas-local pixel — identical in BOTH sessions regardless of how
// much of the viewport each one's panels consume.
const WALL_X = 450;
const WALL_Y0 = 60;
const WALL_Y1 = 560;
const TOKEN_Y = 300;
const PLACE_X = 200;
/** Where the GM parks the token after assigning ownership (the happens-before barrier). */
const HANDOFF_X = 240;
/** Control leg: a legal destination on the token's own side of the wall. */
const LEGAL_X = 340;
/** Rejected leg: past the wall, so the move segment crosses it. */
const ILLEGAL_X = 640;
/** Second legal destination, used as the post-rejection sync barrier. */
const SETTLE_X = 290;

const VIEWPORT = { width: 1600, height: 1000 };

test.use({ viewport: VIEWPORT });

type Point = { x: number; y: number };

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

async function login(page: Page, username: string, password: string): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Username").fill(username);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Log in" }).click();
}

/** Canvas-local → page coordinates. Re-read per gesture: opening or closing a panel
 * resizes the canvas (which moves its origin) without moving the camera. */
async function canvasOrigin(page: Page): Promise<Point> {
  const box = await page.getByTestId("stage-canvas").boundingBox();
  expect(box, "the stage canvas must be laid out before a pointer gesture").not.toBeNull();
  // Gestures below address scene coordinates out to (ILLEGAL_X, WALL_Y1). A canvas
  // that small keeps the pointer inside the element for every gesture, so no gesture
  // depends on Stage's pointer capture retargeting an out-of-canvas move.
  expect(box!.width).toBeGreaterThan(ILLEGAL_X + 20);
  expect(box!.height).toBeGreaterThan(WALL_Y1 + 20);
  return { x: box!.x, y: box!.y };
}

/** One pointermove for the whole displacement.
 *
 * `makeSelectMoveTool` captures the grab origins at `onPointerDown`, so every intent it
 * emits is start→CURRENT — a stepped drag therefore emits progressively longer full-span
 * segments, not incremental hops, and each of the early (short) ones commits. The rollback
 * assertion would then revert to the last committed intermediate rather than to the pre-drag
 * position. A single pointermove still emits two intents (the leading-edge send in
 * `onPointerMove`, then the `onPointerUp` flush), but both carry the identical full
 * displacement, so nothing partial can commit. */
async function dragScene(page: Page, from: Point, to: Point): Promise<void> {
  const o = await canvasOrigin(page);
  await page.mouse.move(o.x + from.x, o.y + from.y);
  await page.mouse.down();
  await page.mouse.move(o.x + to.x, o.y + to.y);
  await page.mouse.up();
}

/** `data-token-positions` is `id:x,y` pairs, id-sorted and `;`-joined (Stage.svelte). */
function parseX(positions: string): number {
  const entries = positions.split(";").filter((s) => s.length > 0);
  expect(entries, "exactly one token is expected on this scene").toHaveLength(1);
  const coords = entries[0].slice(entries[0].indexOf(":") + 1).split(",");
  return Number(coords[0]);
}

async function currentPositions(page: Page): Promise<string> {
  return (await stageHost(page).getAttribute("data-token-positions")) ?? "";
}

/** Record every distinct value `data-token-positions` takes from now on.
 *
 * Polling cannot serve this test: an optimistic write and its rollback are one
 * server round trip apart on a loopback socket, so a sampled assertion would miss
 * the intermediate state and a "the position never crossed the wall" claim would
 * pass even if the drag had never dispatched at all. The observer sees every value. */
async function watchPositions(page: Page): Promise<void> {
  await page.evaluate(() => {
    const host = document.querySelector(".stage-host") as HTMLElement;
    const w = window as unknown as { __posLog: string[] };
    w.__posLog = [host.dataset.tokenPositions ?? ""];
    new MutationObserver(() => {
      const v = host.dataset.tokenPositions ?? "";
      if (w.__posLog[w.__posLog.length - 1] !== v) w.__posLog.push(v);
    }).observe(host, { attributes: true, attributeFilter: ["data-token-positions"] });
  });
}

async function positionLog(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __posLog: string[] }).__posLog);
}

/** Wait for a page to observe the token at (approximately) a scene x. */
async function expectTokenX(page: Page, x: number, why: string): Promise<void> {
  await expect
    .poll(async () => parseX(await currentPositions(page)), { message: why, timeout: 20_000 })
    .toBeCloseTo(x, -1);
}

// The suite's only two-session spec. A GM bypasses the movement gate entirely
// (`ws/room.rs`), so only a genuine non-GM account can prove the server is what stops
// an illegal move — hence the admin-minted account, seated by invite, given player
// tools and a token it effectively owns.
//
// THE CONTROL LEG. `Room::publish` runs the movement gate BEFORE `apply_intent`, so a
// permission denial and a wall denial are the same `Forbidden` and are indistinguishable
// from the client. "The move rolled back" alone would therefore also be produced by an
// ownership regression, proving nothing about the wall. The scene is set to
// `movementRestriction: "unrestricted"`, where `blocks_move` is the ONLY thing that can
// reject a non-GM move, and the same player drags the same token in the same session to a
// LEGAL destination first: that leg must succeed. Only then does a rejection isolate the wall.
//
// SCOPE — this does NOT cover the hex traversal gate. `Unrestricted` makes `Room::publish`
// `continue` before it ever resolves a `GridShape` or walks `line_traversal`, so the server-side
// gate exercised here is the grid-kind-AGNOSTIC `blocks_move` segment check. The scene being hex
// drives the client (render, snapping) and proves the hex authoring path round-trips; the
// hex-specific mask/traversal gate is proven by the server's own unit and integration tests, not
// by this spec. Do not read a change here as changing that coverage.
test("a non-GM player's wall-crossing drag on a hex scene is rejected by the server and rolled back", async ({
  page,
  browser,
}) => {
  test.setTimeout(180_000);

  // Unique per run: the dev server is reused across local runs (`reuseExistingServer`),
  // so a fixed account name would collide with its own previous run. Synthetic only.
  const playerName = `player-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Hex Gate World ${Date.now().toString(36)}`;

  // --- GM session: the seeded admin, who is also the world's owner/GM. ---
  const gm = page;
  await login(gm, "ops", "pw-boot");
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // The Settings panel hosts both admin-tier account creation and the GM's invite
  // minting; it starts launcher-closed.
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();

  // Create the player account through the real admin-gated surface (`POST /api/users`).
  await gm.getByLabel("Account name").fill(playerName);
  await gm.getByLabel("Password", { exact: true }).fill(playerPassword);
  await gm.getByRole("button", { name: "Create account" }).click();
  await expect(gm.getByText(`Created account ${playerName}.`)).toBeVisible({ timeout: 15_000 });

  // Mint a player-role invite. The GM never names the account — the invitee redeems
  // the code from their own session — so this is the only seating path there is.
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  // --- Player session: a second browser context (separate cookie jar). ---
  const playerCtx = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", {
      timeout: 30_000,
    });

    // AppContext's member roster is a session-start snapshot, so the GM must re-enter
    // the world for the freshly seated player to be assignable as a token owner.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // --- GM authors the scene through the real game-settings controls. ---
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();
    await gm.getByLabel("gameSettings.scene.gridKind").selectOption("hex");
    // Unrestricted is what makes the rejection below attributable to the wall: the
    // visibility mask cannot reject in this mode, so `blocks_move` is the only gate left.
    // Set at the WORLD tier, which the scene inherits. The per-scene override control
    // cannot be used: a scene document's `engine.vision` is `null` until something
    // writes it, and `/engine/vision/movementRestriction` cannot descend through null.
    await gm.getByLabel("gameSettings.movementRestriction").selectOption("unrestricted");
    // Pin that it persisted. Silently falling back to `Visible` would move the rejection
    // below onto the visibility mask, re-entering the very ambiguity the control leg closes —
    // and on a lit scene the control leg would still pass, so nothing else would catch it.
    await expect(gm.getByLabel("gameSettings.movementRestriction")).toHaveValue("unrestricted");
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();

    // Snap off: with hex snapping on, the wall's endpoints would snap to two hex
    // centers on staggered rows and the wall would be a slanted segment of unknown x.
    // Off, every authored point is exactly the scene coordinate named here. Grid KIND
    // stays hex — snapping and grid rendering are independent axes.
    const snap = gm.getByTestId("snap-toggle");
    await expect(snap).toHaveAttribute("aria-pressed", "true");
    await snap.click();
    await expect(snap).toHaveAttribute("aria-pressed", "false");

    // Token art.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-assets:panel").click();
    await gm
      .getByTestId("asset-upload")
      .setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);

    // Place the player's token on the near side of the wall.
    await gm.getByTestId("tool-place").click();
    const pick = gm.getByTestId("picker-asset").first();
    await expect(pick).toBeVisible({ timeout: 10_000 });
    await pick.click();
    let origin = await canvasOrigin(gm);
    await gm.mouse.click(origin.x + PLACE_X, origin.y + TOKEN_Y);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-assets:panel").click();

    // A `blocksMove` wall between the token and the illegal destination.
    await gm.getByTestId("tool-wall").click();
    await dragScene(gm, { x: WALL_X, y: WALL_Y0 }, { x: WALL_X, y: WALL_Y1 });
    await expect(stageHost(gm)).toHaveAttribute("data-wall-count", "1", { timeout: 15_000 });

    // Hand the token to the player. The per-token override lives in the Actors panel
    // and needs the token selected, which is the select tool's job.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    origin = await canvasOrigin(gm);
    await gm.mouse.click(origin.x + PLACE_X, origin.y + TOKEN_Y);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();

    // Happens-before barrier: the GM nudges the token and waits for the PLAYER to see it.
    // Frames arrive in sequence order on one socket, so once the player has rendered this
    // move it has necessarily already applied the ownership write that preceded it — the
    // player's drags below cannot race ahead of their own authorization.
    await dragScene(gm, { x: PLACE_X, y: TOKEN_Y }, { x: HANDOFF_X, y: TOKEN_Y });
    await expectTokenX(player, HANDOFF_X, "the player must observe the GM's ownership handoff");
    await expectTokenX(gm, HANDOFF_X, "the GM's own nudge must commit");

    // The GM page is the AUTHORITATIVE observer from here on: it issues no optimistic
    // write of its own, so every value it shows came back from the server.
    await watchPositions(gm);
    await watchPositions(player);
    await player.getByTestId("tool-select").click();

    // --- Control leg: a LEGAL drag by the same player, same token, same session. ---
    await dragScene(player, { x: HANDOFF_X, y: TOKEN_Y }, { x: LEGAL_X, y: TOKEN_Y });
    await expectTokenX(gm, LEGAL_X, "the server must ACCEPT a legal player move (control leg)");
    await expectTokenX(player, LEGAL_X, "the accepted move must stand on the player's client");
    const beforeIllegal = await currentPositions(player);

    // --- Rejected leg: the same drag, but across the wall. ---
    await dragScene(player, { x: LEGAL_X, y: TOKEN_Y }, { x: ILLEGAL_X, y: TOKEN_Y });

    // The optimistic position must return to exactly its pre-drag value.
    await expect
      .poll(() => currentPositions(player), {
        message: "the rejected move must roll back to the pre-drag position",
        timeout: 20_000,
      })
      .toBe(beforeIllegal);

    // …and it must have actually left it first. Without this the assertion above is
    // also satisfied by a drag that never dispatched anything.
    const playerLog = await positionLog(player);
    expect(
      playerLog.some((p) => p.length > 0 && parseX(p) > WALL_X),
      "the player's client must have optimistically written a position past the wall",
    ).toBe(true);

    // Second sync barrier: a legal move the GM observes. Once it lands, the rejected
    // move's round trip is provably complete, so the authoritative log below is final.
    await dragScene(player, { x: LEGAL_X, y: TOKEN_Y }, { x: SETTLE_X, y: TOKEN_Y });
    await expectTokenX(gm, SETTLE_X, "a legal move after the rejection must still commit");

    // The server never committed a position past the wall — the whole point.
    const gmLog = await positionLog(gm);
    expect(gmLog.length).toBeGreaterThan(0);
    for (const p of gmLog) {
      if (p.length === 0) continue;
      expect(parseX(p), `authoritative position ${p} must never cross the wall`).toBeLessThan(
        WALL_X,
      );
    }

    // Re-read the restriction now that many server round trips have completed. The check at
    // authoring time can be satisfied by the optimistic value alone; a write the server had
    // rejected would have rolled back well before this point.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();
    await expect(gm.getByLabel("gameSettings.movementRestriction")).toHaveValue("unrestricted");
  } finally {
    await playerCtx.close();
  }
});
