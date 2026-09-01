import { test, expect, login } from "./fixtures";
import type { Page, Locator } from "@playwright/test";

// Senses e2e: a creature-sense (tremorsense) assignment reveals a grounded token through
// fog on a real player's client, elevation on the target breaks the perception (a flying
// token is not felt through the ground), and an off-ground token renders its elevation
// badge chip. All authoring goes through the real UI (the vision-assignment list editor and
// the token elevation control); observation goes through the stage's read-only debug
// attributes (data-perceived-tokens / data-token-badges), never WebGL pixels.

// A 1×1 PNG used as token art (same fixture the stage/hex suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Scene-space geometry. Snap is toggled OFF in both tests, so a click at a scene coordinate
// authors exactly that position. The default scene (square grid, 100-unit cells, fog on,
// lighting on, environment intensity 0) is pitch dark: a token with only normal vision sees
// nothing, so a perceived target is revealed by the creature sense ALONE.
const LURKER = { x: 210, y: 310 }; // the player-owned token (tremorsense source)
const TARGET = { x: 510, y: 310 }; // 3 cells away — inside the authored 12-cell range

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

/** `data-token-positions` is `id:x,y` pairs, id-sorted and `;`-joined. */
function parsePositions(positions: string): Map<string, Point> {
  const out = new Map<string, Point>();
  for (const entry of positions.split(";").filter((s) => s.length > 0)) {
    const [id, xy] = entry.split(":");
    const [x, y] = xy.split(",").map(Number);
    out.set(id, { x, y });
  }
  return out;
}

/** The id of the token at (approximately) a scene position, or null. Approximate because the
 * canvas origin can be fractional, so a click at an integer scene point lands sub-pixel. */
function tokenIdNear(positions: Map<string, Point>, p: Point): string | null {
  for (const [id, at] of positions) {
    if (Math.hypot(at.x - p.x, at.y - p.y) < 5) return id;
  }
  return null;
}

/** The currently perceived id set on a page's stage (data-perceived-tokens; unset reads as none). */
async function perceivedIds(page: Page): Promise<string[]> {
  const raw = (await stageHost(page).getAttribute("data-perceived-tokens")) ?? "";
  return raw.split(";").filter((s) => s.length > 0);
}

/** Turn grid snapping off so every authored point is exactly the clicked scene coordinate. */
async function disableSnap(page: Page): Promise<void> {
  const snap = page.getByTestId("snap-toggle");
  await expect(snap).toHaveAttribute("aria-pressed", "true");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");
}

/** Upload the 1×1 PNG through the assets panel (real upload pipeline). */
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

test("a tremorsense assignment reveals a grounded token through fog, and raising its elevation ends the perception", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(180_000);

  const playerName = `player-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Senses World ${Date.now().toString(36)}`;

  // --- GM session: the worker's admin account, who is also the world's owner/GM. ---
  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // Account + invite through the real settings surfaces (the hex-gate suite's pattern).
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
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // AppContext's member roster is a session-start snapshot, so the GM re-enters the world
    // for the freshly seated player to appear in the owner select.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await disableSnap(gm);
    await uploadTokenArt(gm);

    // --- Author the player's actor through the real actors panel: linked placement (the
    // "independent copy" checkbox unchecked) so the later vision edit reaches the token. ---
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    const actors = gm.locator(".actors");
    await actors.getByLabel("Name", { exact: true }).fill("Lurker");
    await actors.getByLabel("New independent copy on each placement").uncheck();
    await actors.getByRole("button", { name: "tok.png" }).click();
    await actors.getByRole("button", { name: "Create actor" }).click();
    const row = actors.getByRole("listitem");
    await expect(row).toHaveCount(1);

    // Hand the actor to the player; ownership of a LINKED token resolves through it.
    await row.getByLabel("Owner").selectOption({ label: playerName });

    // Place the player's token, then the raw target token 3 cells away.
    await row.getByRole("button", { name: "Lurker" }).click();
    await gm.getByTestId("tool-place").click();
    let origin = await canvasOrigin(gm);
    await gm.mouse.click(origin.x + LURKER.x, origin.y + LURKER.y);
    const pick = gm.getByTestId("picker-asset").first();
    await expect(pick).toBeVisible({ timeout: 10_000 });
    await pick.click();
    origin = await canvasOrigin(gm);
    await gm.mouse.click(origin.x + TARGET.x, origin.y + TARGET.y);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });

    // Both tokens ride the player's document stream (creature senses pierce fog, never the
    // READ gate — the target is delivered, just not VISIBLE in the dark).
    await expect(stageHost(player)).toHaveAttribute("data-token-count", "2", { timeout: 15_000 });

    // Control: with no creature sense assigned, the player perceives nothing through the fog.
    let targetId: string | null = null;
    await expect
      .poll(
        async () => {
          const positions = parsePositions(
            (await stageHost(player).getAttribute("data-token-positions")) ?? "",
          );
          targetId = tokenIdNear(positions, TARGET);
          return targetId;
        },
        { message: "the player's stage must report both committed token positions", timeout: 15_000 },
      )
      .not.toBeNull();
    expect(targetId, "the target token's id resolved from its committed position").not.toBeNull();
    // The empty-set control is meaningful only once the player's vision pipeline has spoken:
    // wait for the FIRST applied vision frame (the attribute exists) before asserting it.
    await expect
      .poll(
        async () => (await stageHost(player).getAttribute("data-perceived-tokens")) !== null,
        { message: "the player's stage must have applied its first vision frame", timeout: 15_000 },
      )
      .toBe(true);
    expect(await perceivedIds(player)).toEqual([]);

    // --- The reveal: add a tremorsense row to the actor through the per-row editor. ---
    await row.getByTestId("vision-add").click();
    const modeSelect = row.getByTestId("vision-mode-0");
    await expect(modeSelect).toBeVisible();
    await modeSelect.selectOption("tremorsense");
    await row.getByTestId("vision-range-0").fill("12");
    await row.getByTestId("vision-range-0").press("Tab"); // blur commits the change handler

    await expect
      .poll(async () => perceivedIds(player), {
        message: "the tremorsense row must reveal the grounded target through fog",
        timeout: 20_000,
      })
      .toEqual([targetId]);

    // --- The grounding rule: raising the target off the ground ends the perception. ---
    await gm.getByTestId("tool-select").click();
    origin = await canvasOrigin(gm);
    await gm.mouse.click(origin.x + TARGET.x, origin.y + TARGET.y);
    const elevation = actors.getByTestId("token-elevation");
    await expect(elevation).toBeVisible();
    await elevation.fill("10");
    await elevation.press("Tab");

    await expect
      .poll(async () => perceivedIds(player), {
        message: "a flying target must leave the perceived set",
        timeout: 20_000,
      })
      .toEqual([]);

    // The same write renders the target's elevation badge on the GM's stage.
    await expect
      .poll(
        async () => (await stageHost(gm).getAttribute("data-token-badges")) ?? "",
        { message: "the elevated token must carry its badge chip", timeout: 15_000 },
      )
      .toContain(`${targetId}:↑10`);
  } finally {
    await playerCtx.close();
  }
});

test("a token off the ground renders its elevation badge, and returning to ground removes it", async ({
  page,
  account,
}) => {
  const worldName = `Badge World ${Date.now().toString(36)}`;
  const gm = page;
  await login(gm, account.username, account.password);
  await gm.getByLabel("New world name").fill(worldName);
  await gm.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await disableSnap(gm);
  await uploadTokenArt(gm);

  // Place a raw token.
  await gm.getByTestId("tool-place").click();
  const pick = gm.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();
  let origin = await canvasOrigin(gm);
  await gm.mouse.click(origin.x + TARGET.x, origin.y + TARGET.y);
  await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

  // Grounded: no badge chip.
  const badges = async () => (await stageHost(gm).getAttribute("data-token-badges")) ?? "";
  await expect.poll(badges, { timeout: 15_000 }).not.toContain("↑");

  // Select the token and raise it through the token elevation control.
  await gm.getByTestId("launcher-trigger").click();
  await gm.getByTestId("launcher-item-actors:panel").click();
  await gm.getByTestId("tool-select").click();
  origin = await canvasOrigin(gm);
  await gm.mouse.click(origin.x + TARGET.x, origin.y + TARGET.y);
  const elevation = gm.locator(".actors").getByTestId("token-elevation");
  await expect(elevation).toBeVisible();
  await elevation.fill("3");
  await elevation.press("Tab");
  await expect
    .poll(badges, { message: "the elevated token must render its badge", timeout: 15_000 })
    .toContain("↑3");

  // Back to ground (0 normalizes to an absent stored value): the badge disappears.
  await elevation.fill("0");
  await elevation.press("Tab");
  await expect
    .poll(badges, { message: "a grounded token renders no elevation badge", timeout: 15_000 })
    .not.toContain("↑3");
});
