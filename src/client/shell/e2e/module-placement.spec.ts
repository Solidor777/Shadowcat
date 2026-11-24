import { randomUuid } from "@shadowcat/core";
import { test, expect, login, createAccount, openPanel, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Module-declared placements, end to end. The installed fixture module `placement-fixture` (see
// `e2e-support/fixture-modules`) declares three effects in its manifest: `gm-zone` (who `gm`),
// `open-zone` (who `player`) and `cast-zone` (who `caster_owner`). A GM enables it for the world.
// The client sends only the area (`place_effect`); the server authorises it from the manifest. A GM
// places the GM-only area; a player is refused it with the one uniform wording that discloses
// nothing; and the owner of a caster token places the caster-gated area while a request that
// names no caster is refused. The placed region is read back from the documents API (its
// `system.placement` band) and from the stage's region attribute on both sessions.

// A 1x1 PNG used as token art.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

/** The fixture module's folder id, as the module manager labels its checkbox. */
const FIXTURE_MODULE = "placement-fixture";

/** The one refusal wording a non-GM sender receives for every variant. */
const UNIFORM_REJECTION = "placement rejected";

const TOKEN_AT = { x: 210, y: 310 };

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** The outcome of one `place_effect` request: the refusal text, or `"placed"`. */
type PlaceOutcome = string;

/** Sends one `place_effect` frame over a second connection of the page's own session and reports
 * what the server answered: the correlated refusal message, or `"placed"` once the committed
 * placement arrives as an event.
 * @param page A page signed in as the sender.
 * @param worldName The world the sender is in.
 * @param placement The declaration id to place.
 * @param withCaster Whether the request names the world's first token as caster.
 * @returns The refusal message or `"placed"`.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const outcome = await requestPlacement(page, "World", "gm-zone", false);
 * ```
 */
async function requestPlacement(
  page: Page,
  worldName: string,
  placement: string,
  withCaster: boolean,
): Promise<PlaceOutcome> {
  return page.evaluate(
    async ({ worldName: name, placement: decl, withCaster: useCaster, module, requestId }) => {
      const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
      const world = worlds.find((w) => w.name === name);
      if (!world) throw new Error(`world ${name} not listed`);
      const scenes = (await (await fetch(`/api/worlds/${world.id}/documents?type=scene`)).json()) as { id: string }[];
      const tokens = (await (await fetch(`/api/worlds/${world.id}/documents?type=token`)).json()) as { id: string }[];
      return new Promise<string>((resolve, reject) => {
        const ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws?world=${world.id}`);
        const timer = setTimeout(() => {
          ws.close();
          reject(new Error("no answer to place_effect"));
        }, 20_000);
        const finish = (outcome: string): void => {
          clearTimeout(timer);
          ws.close();
          resolve(outcome);
        };
        ws.onopen = () => ws.send(JSON.stringify({ type: "hello", world: world.id, last_seq: null }));
        ws.onmessage = (m) => {
          const raw = String(m.data);
          const frame = JSON.parse(raw) as { type: string; request_id?: string; message?: string };
          if (frame.type === "welcome") {
            ws.send(
              JSON.stringify({
                type: "place_effect",
                request_id: requestId,
                module,
                placement: decl,
                scene_id: scenes[0].id,
                shape: { kind: "circle", points: [400, 400, 50] },
                ...(useCaster ? { caster: tokens[0].id } : {}),
              }),
            );
          } else if (frame.type === "place_effect_error" && frame.request_id === requestId) {
            finish(frame.message ?? "");
          } else if (frame.type === "event" && raw.includes(`"module":"${module}"`) && raw.includes(`"id":"${decl}"`)) {
            finish("placed");
          }
        };
        ws.onerror = () => {
          clearTimeout(timer);
          reject(new Error("websocket error"));
        };
      });
    },
    { worldName, placement, withCaster, module: FIXTURE_MODULE, requestId: randomUuid() },
  );
}

/** The `system.placement` bands of every region document the page's session can read.
 * @param page A page signed in as a member of the world.
 * @param worldName The world's name.
 * @returns The placement bands, in listing order.
 * @example
 * ```
 * declare const page: import("@playwright/test").Page;
 * const bands = await placedRegions(page, "World");
 * ```
 */
async function placedRegions(page: Page, worldName: string): Promise<{ module: string; id: string }[]> {
  return page.evaluate(async (name) => {
    const worlds = (await (await fetch("/api/worlds")).json()) as { id: string; name: string }[];
    const world = worlds.find((w) => w.name === name);
    if (!world) throw new Error(`world ${name} not listed`);
    const regions = (await (await fetch(`/api/worlds/${world.id}/documents?type=region`)).json()) as {
      system?: { placement?: { module: string; id: string } };
    }[];
    return regions.flatMap((r) => (r.system?.placement ? [r.system.placement] : []));
  }, worldName);
}

test("module placement: the GM places a declared area, a player is refused it uniformly, and a caster's owner places the caster-gated one", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Placement World ${Date.now().toString(36)}`;

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

    // The member roster is a session-start snapshot: re-enter so the player can be named an owner.
    await gm.getByRole("button", { name: /leave world/i }).click();
    await gm.getByRole("button", { name: new RegExp(worldName) }).click();
    await expect(stageHost(gm)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // A token on the stage, owned by the player.
    const snap = gm.getByTestId("snap-toggle");
    await expect(snap).toHaveAttribute("aria-pressed", "true");
    await snap.click();
    await openPanel(gm, "asset-browser:panel");
    await gm.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
    await expect(gm.getByTestId("asset-tile")).toHaveCount(1);
    await openPanel(gm, "asset-browser:panel");
    await gm.getByTestId("tool-place").click();
    const pick = gm.getByTestId("picker-asset").first();
    await expect(pick).toBeVisible({ timeout: 10_000 });
    await pick.click();
    await clickScene(gm, TOKEN_AT);
    await expect(stageHost(gm)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
    await openPanel(gm, "actors:panel");
    await gm.getByTestId("tool-select").click();
    await clickScene(gm, TOKEN_AT);
    await gm.getByLabel("Token owner").selectOption({ label: playerName });
    await expect(gm.getByText(`Effective owner: ${playerName}`)).toBeVisible({ timeout: 15_000 });

    // Control: before the module is enabled for the world, the declaration does not exist.
    expect(await requestPlacement(gm, worldName, "gm-zone", false)).toBe("no such enabled placement");

    // Enable the installed fixture module for the world through the module manager.
    await openPanel(gm, "settings:panel");
    const manager = gm.locator(".module-manager");
    await expect(manager).toBeVisible();
    await manager.getByRole("checkbox", { name: FIXTURE_MODULE }).check();
    await manager.getByRole("button", { name: "Save" }).click();

    // The GM places the GM-only area: it is refused until the save lands, then committed with its
    // manifest-derived placement band.
    await expect
      .poll(() => requestPlacement(gm, worldName, "gm-zone", false), { timeout: 30_000 })
      .toBe("placed");
    await expect.poll(() => placedRegions(gm, worldName)).toEqual([{ module: FIXTURE_MODULE, id: "gm-zone", by: expect.any(String) }]);
    await expect(stageHost(gm)).toHaveAttribute("data-region-shapes", /:circle/);
    await expect(stageHost(player)).toHaveAttribute("data-region-shapes", /:circle/, { timeout: 15_000 });

    // A player is refused the GM-only area with the uniform wording, and no region is added.
    expect(await requestPlacement(player, worldName, "gm-zone", false)).toBe(UNIFORM_REJECTION);
    // The caster-gated area without a caster is refused with the same wording.
    expect(await requestPlacement(player, worldName, "cast-zone", false)).toBe(UNIFORM_REJECTION);
    expect(await placedRegions(gm, worldName)).toHaveLength(1);

    // The owner of the caster token places the caster-gated area.
    expect(await requestPlacement(player, worldName, "cast-zone", true)).toBe("placed");
    await expect.poll(async () => (await placedRegions(gm, worldName)).map((p) => p.id).sort()).toEqual(["cast-zone", "gm-zone"]);
  } finally {
    await playerCtx.close();
  }
});
