import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene, dragScene } from "./stage-gestures";
import type { ScenePoint as Point } from "./stage-gestures";

// Multi-destination portal e2e: a GM authors a `portal_choice` region with two same-scene
// destinations, hands a token to a player, and the player walks it into the region. The
// player's `PortalChooser` lists both option labels; picking one moves the token to that
// option's authored point; walking in again and pressing Escape declines, leaving the token
// where it stands. Both option destinations are the portal's own scene (`PortalTarget.scene:
// null`), the same-scene authoring path `levels.spec.ts` documents. The GM connection is also
// addressed by every offer, so the GM's chooser appearing is asserted as the second viewer.

const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

const PLAYER_START = { x: 210, y: 310 };
const REGION_A = { x: 350, y: 260 };
const REGION_B = { x: 450, y: 360 }; // a 100x100 box
const WALK_IN = { x: 400, y: 310 }; // inside the region box
const WALK_OUT = { x: 250, y: 310 }; // outside it again
const DEST_LIBRARY = { x: 700, y: 700 };
const DEST_CELLAR = { x: 900, y: 700 };

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
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

/** The first token's position on a page's stage, or null. */
async function onlyTokenPosition(page: Page): Promise<Point | null> {
  const positions = parsePositions((await stageHost(page).getAttribute("data-token-positions")) ?? "");
  return positions.values().next().value ?? null;
}

/** Fill one option row: label, destination x/y, committed on blur like the teleport editor. */
async function fillOption(gm: Page, index: number, label: string, dest: Point): Promise<void> {
  const row = gm.getByTestId("region-trigger-portal-option").nth(index);
  await row.getByTestId("region-trigger-portal-label").fill(label);
  await row.getByTestId("region-trigger-teleport-x").fill(String(dest.x));
  await row.getByTestId("region-trigger-teleport-y").fill(String(dest.y));
  await row.getByTestId("region-trigger-teleport-y").blur();
}

test("portal choice: the owner picks a destination, a decline stays put, and the GM is offered too", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Portal World ${Date.now().toString(36)}`;

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

  const playerCtx = await browser.newContext({ baseURL: test.info().project.use.baseURL, viewport: VIEWPORT });
  const player = await playerCtx.newPage();
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // Snap off so every authored point is exactly the clicked scene coordinate.
    const snap = gm.getByTestId("snap-toggle");
    await expect(snap).toHaveAttribute("aria-pressed", "true");
    await snap.click();

    // Unrestricted movement: the placed token carries no vision source, so the default
    // restriction would refuse the player's route regardless of the portal.
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();
    await gm.getByLabel("Movement restriction", { exact: true }).selectOption("unrestricted");
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-game-settings:panel").click();

    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "token.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-asset-browser:panel").click();

    await gm.getByTestId("tool-place").click();
    const pick = gm.getByTestId("picker-asset").first();
    await expect(pick).toBeVisible({ timeout: 10_000 });
    await pick.click();
    await clickScene(gm, PLAYER_START);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

    await gm.getByTestId("launcher-trigger").click();
    await gm.getByTestId("launcher-item-actors:panel").click();
    await gm.getByTestId("tool-select").click();
    await clickScene(gm, PLAYER_START);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });

    // Author the portal: "Several destinations" with two labelled options.
    await gm.getByTestId("tool-region").click();
    await gm.getByTestId("region-trigger-add").click();
    await gm.getByTestId("region-trigger-effect").selectOption("teleport");
    await gm.getByTestId("region-trigger-portal-several").check();
    await fillOption(gm, 0, "The Library", DEST_LIBRARY);
    await gm.getByTestId("region-trigger-portal-add").click();
    await fillOption(gm, 1, "The Cellar", DEST_CELLAR);
    await dragScene(gm, REGION_A, REGION_B);

    // The player walks in: the chooser lists both labels.
    await player.getByTestId("tool-select").click();
    await dragScene(player, PLAYER_START, WALK_IN);
    const chooser = player.getByTestId("portal-chooser");
    await expect(chooser).toBeVisible({ timeout: 20_000 });
    await expect(chooser.getByTestId("portal-option")).toHaveText([/The Library/, /The Cellar/]);

    // The GM connection is addressed by the same offer.
    await expect(gm.getByTestId("portal-chooser")).toBeVisible({ timeout: 20_000 });

    // Picking "The Library" moves the token to that option's point.
    await chooser.getByTestId("portal-option").first().click();
    await expect(chooser).not.toBeVisible();
    await expect
      .poll(
        async () => {
          const at = await onlyTokenPosition(player);
          return at ? Math.hypot(at.x - DEST_LIBRARY.x, at.y - DEST_LIBRARY.y) < 5 : false;
        },
        { message: "the token must land at the chosen option's point", timeout: 20_000 },
      )
      .toBe(true);
    // The answered offer is gone from the GM's chooser too only once the GM answers or it
    // expires; the GM declines it so the next offer is the one asserted below.
    if (await gm.getByTestId("portal-chooser").isVisible()) await gm.keyboard.press("Escape");

    // Walk back out and in again: Escape declines and the token stays where it stands.
    // The portal hop ended the walk-in glide, so the first drag away is accepted at once.
    await dragScene(player, DEST_LIBRARY, WALK_OUT);
    await expect
      .poll(async () => {
        const at = await onlyTokenPosition(player);
        return at ? Math.hypot(at.x - WALK_OUT.x, at.y - WALK_OUT.y) < 60 : false;
      }, { timeout: 20_000 })
      .toBe(true);
    // The walk-out's own glide holds the moving-lock, so the walk-in repeats until the
    // offer appears.
    await expect(async () => {
      await dragScene(player, WALK_OUT, WALK_IN);
      await expect(chooser).toBeVisible({ timeout: 5_000 });
    }).toPass({ timeout: 30_000 });
    const before = await onlyTokenPosition(player);
    await player.keyboard.press("Escape");
    await expect(chooser).not.toBeVisible();
    const after = await onlyTokenPosition(player);
    expect(after).toEqual(before);
  } finally {
    await playerCtx.close();
  }
});
