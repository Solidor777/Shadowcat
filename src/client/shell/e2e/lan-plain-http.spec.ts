import { test, expect, login, createAccount, DUAL_SESSION_TIMEOUT_MS, newE2EContext } from "./fixtures";
import type { Page } from "@playwright/test";
import { clickScene, dragScenePath, sceneCenter, sceneOrigin } from "./stage-gestures";

// Plain-HTTP LAN play. This file runs under the `lan-plain-http` Playwright project, which serves
// the app from a non-loopback hostname mapped to the loopback test server, so the browser itself
// classifies the origin as NOT a secure context and withholds every `[SecureContext]` API. Each
// session drives the flows that mint client ids and request ids: world entry, actor authoring,
// live search, token placement, drawing, combat request ids, chat, registry editors, level
// authoring, custom themes, panel pop-out, invites, and the shared ruler. Any uncaught page error
// fails the test, so the first throwing call is named in the failure.

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

// A 1x1 PNG used as token art.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

/** Every session's uncaught page errors for the running test, drained by the `afterEach` below. */
const sessionErrors: string[][] = [];

/** Records every uncaught page error (popup windows included, via the context) so a
 * secure-context-only call that throws inside an event handler is reported by name. The
 * `afterEach` assertion reports them even when the test failed earlier, so a session that never
 * reached its first assertion still names the call that threw.
 * @param page The session's page.
 */
function collectPageErrors(page: Page): void {
  // `weberror` fires for every page in the context, popups included.
  const errors: string[] = [];
  sessionErrors.push(errors);
  page.context().on("weberror", (e) => errors.push(e.error().stack ?? e.error().message));
}

test.afterEach(() => {
  const all = sessionErrors.splice(0).flat();
  expect(all, "uncaught page errors").toEqual([]);
});

/** Positive control: the page really is an insecure origin. `isSecureContext` is the browser's
 * own verdict, so a harness that silently regained a secure origin fails here. `getRandomValues`
 * is the one crypto member that is NOT `[SecureContext]`, so it stays present. Any raw call to a
 * withheld member would throw in the flows below, which is what drives the ids through
 * `randomUuid`.
 * @param page The page to probe.
 */
async function assertLanShape(page: Page): Promise<void> {
  const shape = await page.evaluate(() => ({
    isSecureContext: window.isSecureContext,
    getRandomValues: typeof crypto.getRandomValues,
  }));
  expect(shape).toEqual({ isSecureContext: false, getRandomValues: "function" });
}

/** Members recorded per owner object; every entry is `[SecureContext]` in its Web IDL. */
const SECURE_CONTEXT_MEMBERS = {
  crypto: ["subtle"],
  navigator: ["mediaDevices", "clipboard", "serviceWorker", "storage", "locks", "share", "credentials", "deviceMemory", "gpu"],
  audioContext: ["audioWorklet"],
} as const;

test("measured: which browser APIs an insecure origin withholds", async ({ page }) => {
  await page.goto("/");
  const measured = await page.evaluate((members) => {
    const context = new AudioContext();
    try {
      const owners: Record<string, Record<string, unknown>> = {
        crypto: crypto as unknown as Record<string, unknown>,
        navigator: navigator as unknown as Record<string, unknown>,
        audioContext: context as unknown as Record<string, unknown>,
      };
      const w = window as unknown as Record<string, unknown>;
      const out: Record<string, string> = { isSecureContext: String(window.isSecureContext) };
      for (const [owner, names] of Object.entries(members)) {
        for (const name of names) out[`${owner}.${name}`] = typeof owners[owner][name];
      }
      out["crypto.getRandomValues"] = typeof crypto.getRandomValues;
      out["navigator.hardwareConcurrency"] = typeof navigator.hardwareConcurrency;
      for (const name of ["Notification", "AudioContext", "AudioWorkletNode", "BroadcastChannel", "localStorage", "WebAssembly", "caches", "indexedDB"]) {
        out[name] = typeof w[name];
      }
      return out;
    } finally {
      void context.close();
    }
  }, SECURE_CONTEXT_MEMBERS);
  test.info().attach("measured-apis", { body: JSON.stringify(measured, null, 2), contentType: "application/json" });
  expect(measured.isSecureContext).toBe("false");
  expect(measured["crypto.getRandomValues"]).toBe("function");
  for (const [owner, names] of Object.entries(SECURE_CONTEXT_MEMBERS)) {
    for (const name of names) expect(measured[`${owner}.${name}`], `${owner}.${name}`).toBe("undefined");
  }
});

test("a GM session without secure-context APIs authors, searches, places, draws, fights, chats and pops out", async ({
  page,
  account,
}) => {
  collectPageErrors(page);

  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(`Lan World ${Date.now().toString(36)}`);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
  await assertLanShape(page);

  // Token art upload.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await page.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Actor authoring (scene-docs `envelope` id) then live search (correlated `request_id`).
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-actors:panel").click();
  const actors = page.locator(".actors");
  await actors.getByPlaceholder("Name", { exact: true }).fill("Lanternbearer");
  await actors.getByTestId("visual-pick").click();
  const pickDialog = page.getByTestId("asset-pick-dialog");
  await expect(pickDialog).toBeVisible();
  await pickDialog.getByTestId("asset-tile").first().click();
  await pickDialog.getByTestId("pick-confirm").click();
  await expect(pickDialog).toHaveCount(0);
  await actors.getByRole("button", { name: "Create actor" }).click();
  await expect(actors.getByRole("button", { name: "Lanternbearer", exact: true })).toBeVisible({ timeout: 15_000 });
  await actors.getByLabel("Search actors").fill("Lantern");
  await expect(actors.getByRole("button", { name: "Lanternbearer", exact: true })).toBeVisible({ timeout: 15_000 });
  await actors.getByLabel("Search actors").fill("zzzz-no-such-actor");
  await expect(actors.getByRole("button", { name: "Lanternbearer", exact: true })).toHaveCount(0, { timeout: 15_000 });
  await actors.getByLabel("Search actors").fill("");

  // Token placement (the place tool's `envelope` id) and drawing.
  await page.getByTestId("tool-place").click();
  const pick = page.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();
  const center = await sceneCenter(page);
  await clickScene(page, center);
  await expect(stageHost(page)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

  await page.getByTestId("tool-draw").click();
  await dragScenePath(page, { x: center.x - 40, y: center.y + 80 }, [{ x: center.x, y: center.y + 50 }, { x: center.x + 40, y: center.y + 80 }], 3);
  await expect(stageHost(page)).toHaveAttribute("data-shape-count", "1", { timeout: 15_000 });

  // Combat: create + add the selected token (client doc ids), then Start (`combat_start` request id).
  await page.getByTestId("tool-select").click();
  await clickScene(page, center);
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-combat-tracker:panel").click();
  await page.getByTestId("combat-tracker:create").click();
  await expect(page.getByTestId("combat-tracker:add-selected")).toBeEnabled({ timeout: 15_000 });
  await page.getByTestId("combat-tracker:add-selected").click();
  await page.getByRole("button", { name: "Start", exact: true }).click();
  await expect(page.getByText("Round 1", { exact: true })).toBeVisible({ timeout: 15_000 });

  // Chat (`SendMessage` request id).
  const line = `lan hello ${Date.now().toString(36)}`;
  await page.locator("#chat-composer-input").fill(line);
  await page.locator("#chat-composer-input").press("Enter");
  await expect(page.getByText(line)).toBeVisible({ timeout: 15_000 });

  // Registry editors (`randomUuid` entry keys).
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-factions:panel").click();
  await page.getByRole("button", { name: "Add faction" }).click();
  await expect(page.getByRole("button", { name: "Remove", exact: true }).first()).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-conditions:panel").click();
  await page.getByRole("button", { name: "Add condition" }).click();
  await expect(page.getByRole("button", { name: "Remove", exact: true }).first()).toBeVisible({ timeout: 15_000 });

  // Level authoring (`LevelsEditor` id).
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-scene-browser:panel").click();
  await page.getByTestId("levels-toggle").first().click();
  const editor = page.getByTestId("levels-editor");
  await editor.getByTestId("level-add").click();
  await expect(editor.getByTestId("level-row")).toHaveCount(1);
  expect(await editor.getByTestId("level-row").first().getAttribute("data-level-id")).toMatch(UUID_V4);

  // Custom theme (`ThemeEditor` id): saving persists a `custom:<id>` active theme.
  await page.getByTestId("topbar-settings").click();
  await page.getByRole("button", { name: "New custom theme" }).click();
  await page.getByLabel("Theme name").fill("Lan Theme");
  const persisted = page.waitForResponse((r) => {
    if (!/\/api\/me\/ui-state$/.test(r.url()) || r.request().method() !== "PUT" || !r.ok()) return false;
    try {
      const body = r.request().postDataJSON() as { global?: { theme?: { active?: string } } };
      return body?.global?.theme?.active?.startsWith("custom:") === true;
    } catch {
      return false;
    }
  });
  await page.getByRole("button", { name: "Save theme" }).click();
  await persisted;

  // Microphone ducking source: the platform withholds its media-device API, so enabling it
  // reports the unavailable reason instead of failing opaquely.
  // The engine's `AudioContext` must be unlocked before the toggle can reach the source; a denial
  // reverts the checkbox, so the toggle is clicked, not `check()`ed.
  await page.getByTestId("audio-unlock").click();
  await page.getByLabel("Duck when I speak").click();
  await expect(page.getByRole("alert").filter({ hasText: "secure connection" })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByLabel("Duck when I speak")).not.toBeChecked();

  // Pop-out (`dockview` window key) opens a popup window carrying the same stripped APIs.
  const popupEvent = page.context().waitForEvent("page");
  await page.locator(".dv-tab", { hasText: "Chat" }).first().locator(".sc-tab-menu-btn").click();
  await page.getByTestId("panel-menu-popOut").click();
  const popup = await popupEvent;
  await popup.waitForLoadState();
  expect(popup.url()).toContain("popout.html");
  await assertLanShape(popup);

});

test("a player session without secure-context APIs joins by invite, chats and shares a ruler", async ({
  page,
  browser,
  account,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  collectPageErrors(page);

  const playerName = `player-${test.info().workerIndex}-${Date.now().toString(36)}`;
  const playerPassword = "pw-player-e2e";
  const worldName = `Lan Join World ${Date.now().toString(36)}`;

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

  const playerCtx = await newE2EContext(browser, { baseURL: test.info().project.use.baseURL, viewport: VIEWPORT });
  const player = await playerCtx.newPage();
  collectPageErrors(player);
  try {
    await login(player, playerName, playerPassword);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
    await assertLanShape(player);

    const line = `player lan hello ${Date.now().toString(36)}`;
    await player.locator("#chat-composer-input").fill(line);
    await player.locator("#chat-composer-input").press("Enter");
    await expect(player.getByText(line)).toBeVisible({ timeout: 15_000 });
    await expect(gm.getByText(line)).toBeVisible({ timeout: 15_000 });

    await player.getByTestId("tool-measure").click();
    await player.getByTestId("share-ruler-toggle").click();
    const o = await sceneOrigin(player, { x: 200, y: 300 });
    await player.mouse.down();
    await player.mouse.move(o.x + 300, o.y + 50, { steps: 4 });
    await expect(stageHost(gm)).toHaveAttribute("data-last-ruler", /^[^:]+:true:2$/, { timeout: 15_000 });
    await player.mouse.up();
  } finally {
    await playerCtx.close();
  }
});
