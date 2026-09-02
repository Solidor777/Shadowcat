import { test, expect, login } from "./fixtures";
import type { Page, Locator } from "@playwright/test";

// Moving-light e2e: a GM walks a torch-bearing token (a carried `LightEmission` authored
// through the actors panel) past an observing player's token in a pitch-dark scene. The
// observer's lighting overlay must change WHILE the walk plays — driven by the per-recipient
// `mover_light` timeline and the client's lighting sweep — not only once the token stops; and
// an observer walled off from the whole walk (no line of sight, no glow reaching in) must never
// see a sweep at all. Authoring goes through the real UI; observation goes through the stage's
// read-only debug attributes (`data-light-sweep`, `data-lit-bbox`, `data-lit-cells`), never
// WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Scene-space geometry (square grid, 100-unit cells, snap OFF so a click authors exactly that
// position). The default scene is fog on, lighting on, environment intensity 0: pitch dark, so
// the only light anywhere is the bearer's carried torch (`DEFAULT_LIGHT_EMISSION`: bright 2
// cells, dim 6 cells, linear falloff → a 600-unit dim reach). The COMMITTED lit set admits a
// cell only where the falloff level reaches normal vision's "dim" floor (0.34):
// `200 + (1 - 0.34) × 400 = 464` units from the torch; the client's mid-walk sweep paints the
// full dim disc (a cosmetic approximation), so its westmost column is computed separately.
const OBSERVER = { x: 210, y: 310 };
const START = { x: 1010, y: 310 }; // 8 cells east of the observer; authored exactly here
/** Route goal; the pathfinder snaps every waypoint but the first to a cell center, so the torch
 * comes to rest at (650, 350). */
const GOAL = { x: 610, y: 310 };
/** Westmost COMMITTED lit column with the torch at START: row centers y=350 (dy 40) admit
 * `x ≥ 1010 - sqrt(464² - 40²) ≈ 548` → column 5. */
const START_MIN_COL = 5;
/** Westmost column of the client SWEEP's first sample (the full 600-unit disc from START):
 * `x ≥ 1010 - sqrt(600² - 40²) ≈ 411` → column 4. A sweep frame further west than this proves
 * the torch MOVED before the walk committed. */
const SWEEP_START_MIN_COL = 4;
/** Westmost committed lit column once the torch rests at (650, 350): `x ≥ 650 - 464 = 186` →
 * column 2. */
const GOAL_MIN_COL = 2;

// The walled-off observer of the second test sits inside a closed box; the walk runs outside.
const BOX = { x0: 60, y0: 160, x1: 360, y1: 460 };
const FAR_START = { x: 1510, y: 310 };
const FAR_GOAL = { x: 1110, y: 310 };
/** The route goal is grid-snapped by the server's pathfinder (every waypoint but the first is a
 * cell center), so the committed stop is FAR_GOAL's cell center, not the raw click. */
const FAR_STOP = { x: 1150, y: 350 };

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

type Point = { x: number; y: number };

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** Canvas-local → page coordinates. Re-read per gesture: opening or closing a panel resizes
 * the canvas (which moves its origin) without moving the camera. */
async function canvasOrigin(page: Page): Promise<Point> {
  const box = await page.getByTestId("stage-canvas").boundingBox();
  expect(box, "the stage canvas must be laid out before a pointer gesture").not.toBeNull();
  return { x: box!.x, y: box!.y };
}

/** Turn grid snapping off so every authored point is exactly the clicked scene coordinate. */
async function disableSnap(page: Page): Promise<void> {
  const snap = page.getByTestId("snap-toggle");
  await expect(snap).toHaveAttribute("aria-pressed", "true");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");
}

/** Upload the 1×1 PNG through the assets panel (real upload pipeline), then close the panel. */
async function uploadTokenArt(page: Page): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await page.getByTestId("asset-upload").setInputFiles({
    name: "tok.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
}

/** One pointermove for the whole displacement (the wall tool authors a segment per drag). */
async function dragScene(page: Page, from: Point, to: Point): Promise<void> {
  const o = await canvasOrigin(page);
  await page.mouse.move(o.x + from.x, o.y + from.y);
  await page.mouse.down();
  await page.mouse.move(o.x + to.x, o.y + to.y);
  await page.mouse.up();
}

/** Record every distinct `(data-light-sweep, data-lit-bbox)` pair the observer's stage takes
 * from now on. Polling cannot serve this: the walk plays in well under a second, so a sampled
 * assertion could miss every mid-walk frame. The observer sees every value. */
async function watchLighting(page: Page): Promise<void> {
  await page.evaluate(() => {
    const host = document.querySelector(".stage-host") as HTMLElement;
    const w = window as unknown as { __lightLog: string[] };
    const read = (): string => `${host.dataset.lightSweep ?? ""}|${host.dataset.litBbox ?? ""}`;
    w.__lightLog = [read()];
    new MutationObserver(() => {
      const v = read();
      if (w.__lightLog[w.__lightLog.length - 1] !== v) w.__lightLog.push(v);
    }).observe(host, {
      attributes: true,
      attributeFilter: ["data-light-sweep", "data-lit-bbox"],
    });
  });
}

async function lightLog(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __lightLog: string[] }).__lightLog);
}

/** The westmost lit column of a `data-lit-bbox` value (`minI,minJ,maxI,maxJ`), or null. */
function minCol(bbox: string): number | null {
  if (!bbox) return null;
  const n = Number(bbox.split(",")[0]);
  return Number.isFinite(n) ? n : null;
}

/** GM setup shared by both tests: world, player account + invite, player session seated, GM
 * re-entered (so the owner select sees the player), snap off, token art uploaded, and a LINKED
 * player-owned "Watcher" actor placed at `observerAt` plus a LINKED torch-carrying "Bearer"
 * actor placed at `bearerAt`. Returns the player's page and its context closer. */
async function setupTorchScene(
  gm: Page,
  browser: import("@playwright/test").Browser,
  account: { username: string; password: string },
  worldName: string,
  observerAt: Point,
  bearerAt: Point,
): Promise<{ player: Page; close: () => Promise<void> }> {
  const playerName = `player-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";

  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-settings:panel").click();
  await gm.getByLabel("Account name").fill(playerName);
  await gm.getByLabel("Password", { exact: true }).fill(playerPassword);
  await gm.getByRole("button", { name: "Create account" }).click();
  await expect(gm.getByText(`Created account ${playerName}.`)).toBeVisible({ timeout: 15_000 });
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await browser.newContext({
    baseURL: test.info().project.use.baseURL,
    viewport: VIEWPORT,
  });
  const player = await playerCtx.newPage();
  await login(player, playerName, playerPassword);
  await player.getByLabel("Invite code").fill(code);
  await player.getByRole("button", { name: "Join with an invite code" }).click();
  await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // AppContext's member roster is a session-start snapshot, so the GM re-enters the world for
  // the freshly seated player to appear in the owner select.
  await gm.getByRole("button", { name: /leave world/i }).click();
  await gm.getByRole("button", { name: new RegExp(worldName) }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await disableSnap(gm);
  await uploadTokenArt(gm);

  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-actors:panel").click();
  const actors = gm.locator(".actors");
  // Linked placements (the "independent copy" checkbox unchecked once, for both actors) so the
  // observer's ownership and the bearer's carried light both resolve through the actor.
  await actors.getByLabel("New independent copy on each placement").uncheck();

  await actors.getByLabel("Name", { exact: true }).fill("Watcher");
  await actors.getByRole("button", { name: "tok.png" }).click();
  await actors.getByRole("button", { name: "Create actor" }).click();
  const watcher = actors.getByRole("listitem").filter({ hasText: "Watcher" });
  await expect(watcher).toHaveCount(1);
  await watcher.getByLabel("Owner").selectOption({ label: playerName });

  await actors.getByLabel("Name", { exact: true }).fill("Bearer");
  await actors.getByRole("button", { name: "tok.png" }).click();
  await actors.getByRole("button", { name: "Create actor" }).click();
  const bearer = actors.getByRole("listitem").filter({ hasText: "Bearer" });
  await expect(bearer).toHaveCount(1);
  // The carried light: the per-row toggle stamps `DEFAULT_LIGHT_EMISSION` onto the actor.
  await bearer.getByLabel("Carried light").check();
  await expect(bearer.getByLabel("Carried light")).toBeChecked();

  await watcher.getByRole("button", { name: "Watcher", exact: true }).click();
  await gm.getByTestId("tool-place").click();
  let origin = await canvasOrigin(gm);
  await gm.mouse.click(origin.x + observerAt.x, origin.y + observerAt.y);
  await bearer.getByRole("button", { name: "Bearer", exact: true }).click();
  await gm.getByTestId("tool-place").click();
  origin = await canvasOrigin(gm);
  await gm.mouse.click(origin.x + bearerAt.x, origin.y + bearerAt.y);
  await expect(stageHost(gm)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });
  await expect(stageHost(player)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });
  // Close the actors panel so the canvas origin stays put for the gestures below.
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-actors:panel").click();

  return { player, close: () => playerCtx.close() };
}

/** Walk the bearer from `from` to `to` as the GM: select it, then double-click the goal with
 * the measure tool — the route commit goes through `moveRequest` → `Room::execute_move`, the
 * only path that broadcasts a `MoveStream` (a GM's select-tool drag is a raw position write). */
async function walkBearer(gm: Page, from: Point, to: Point): Promise<void> {
  await gm.getByTestId("tool-select").click();
  let origin = await canvasOrigin(gm);
  await gm.mouse.click(origin.x + from.x, origin.y + from.y);
  await gm.getByTestId("tool-measure").click();
  origin = await canvasOrigin(gm);
  await gm.mouse.dblclick(origin.x + to.x, origin.y + to.y);
}

test("a carried torch lights the corridor for an observing player mid-walk", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(180_000);
  const gm = page;
  const { player, close } = await setupTorchScene(
    gm,
    browser,
    account,
    `Torch World ${Date.now().toString(36)}`,
    OBSERVER,
    START,
  );
  try {
    // The observer's committed lighting: the torch at rest at START lights the corridor's far
    // end (westmost lit column 5), all of it inside the observer's open line of sight.
    await expect
      .poll(async () => minCol((await stageHost(player).getAttribute("data-lit-bbox")) ?? ""), {
        message: "the resting torch must light the observer's view of the corridor",
        timeout: 20_000,
      })
      .toBe(START_MIN_COL);
    await expect(stageHost(player)).toHaveAttribute("data-light-sweep", "0");

    await watchLighting(player);
    await walkBearer(gm, START, GOAL);

    // The move commits its stop first; the observer's stage reports the final position. The
    // lighting, however, must have swept: some logged value carries `data-light-sweep = 1`
    // with the lit columns already west of the resting set — the corridor lit up BEFORE the
    // walk ended, not only when the post-commit lighting frame landed.
    await expect
      .poll(async () => (await stageHost(player).getAttribute("data-light-sweep")) ?? "", {
        message: "the observer's sweep must end once the walk completes",
        timeout: 20_000,
      })
      .toBe("0");
    await expect
      .poll(async () => minCol((await stageHost(player).getAttribute("data-lit-bbox")) ?? ""), {
        message: "the committed lighting after the walk lights up to the goal",
        timeout: 20_000,
      })
      .toBe(GOAL_MIN_COL);
    const log = await lightLog(player);
    const midWalk = log.filter((v) => v.startsWith("1|"));
    expect(midWalk.length, `the observer must have painted a light sweep: ${log.join(" ; ")}`).toBeGreaterThan(0);
    expect(
      midWalk.some((v) => {
        const c = minCol(v.slice(2));
        return c !== null && c < SWEEP_START_MIN_COL;
      }),
      `a sweep frame must light columns west of the resting set: ${midWalk.join(" ; ")}`,
    ).toBe(true);
  } finally {
    await close();
  }
});

test("an observer walled off from the walk never sees a light sweep", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(180_000);
  const gm = page;
  const { player, close } = await setupTorchScene(
    gm,
    browser,
    account,
    `Walled Torch World ${Date.now().toString(36)}`,
    OBSERVER,
    FAR_START,
  );
  try {
    // Box the observer in with four sight-blocking walls (the wall tool authors all three
    // block flags): its line of sight ends at the box, the walk runs far outside it, and the
    // torch's 600-unit glow never reaches the box from its snapped stop (1150 - 600 > 360).
    await gm.getByTestId("tool-wall").click();
    await dragScene(gm, { x: BOX.x0, y: BOX.y0 }, { x: BOX.x0, y: BOX.y1 }); // west
    await dragScene(gm, { x: BOX.x0, y: BOX.y0 }, { x: BOX.x1, y: BOX.y0 }); // north
    await dragScene(gm, { x: BOX.x1, y: BOX.y0 }, { x: BOX.x1, y: BOX.y1 }); // east
    await dragScene(gm, { x: BOX.x0, y: BOX.y1 }, { x: BOX.x1, y: BOX.y1 }); // south
    await expect(stageHost(gm)).toHaveAttribute("data-wall-count", "4", { timeout: 15_000 });
    await expect(stageHost(player)).toHaveAttribute("data-wall-count", "4", { timeout: 15_000 });
    // The boxed observer's committed lighting is empty (its view is a dark box), and no sweep.
    await expect(stageHost(player)).toHaveAttribute("data-lit-cells", "0", { timeout: 20_000 });
    await expect(stageHost(player)).toHaveAttribute("data-light-sweep", "0");

    await watchLighting(player);
    await walkBearer(gm, FAR_START, FAR_GOAL);

    // The GM observes the committed stop, so the walk's round trip is complete; the observer's
    // lighting log must contain no sweep frame and no lit cell.
    await expect
      .poll(async () => (await stageHost(gm).getAttribute("data-token-positions")) ?? "", {
        message: "the GM must observe the bearer's committed stop",
        timeout: 20_000,
      })
      .toContain(`${FAR_STOP.x},${FAR_STOP.y}`);
    await gm.waitForTimeout(1_500); // longer than the walk's playback + the vision debounce
    const log = await lightLog(player);
    expect(log.some((v) => v.startsWith("1|")), `no sweep may reach the boxed observer: ${log.join(" ; ")}`).toBe(false);
    await expect(stageHost(player)).toHaveAttribute("data-lit-cells", "0");
  } finally {
    await close();
  }
});
