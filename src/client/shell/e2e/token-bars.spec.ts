import { test, expect, login } from "./fixtures";
import type { Page, Locator } from "@playwright/test";
import { clickScene } from "./stage-gestures";

// Token nameplate and attribute-bar authoring e2e: the GM authors an actor with a nameplate mode
// and one bar through the actors panel, places a linked token, then switches the token to its own
// bar list through the per-token override control. The drawn plate is canvas output with no DOM
// observable, so the test reads the value the drawing code reads: the delivered `"bars"` derived
// channel frame, whose entry for the token holds the nameplate mode and each bar's current, max
// and mode (the renderer draws a nameplate or bar only when that entry says so). It also covers the
// authoring surfaces and their persistence (the editors re-read the stored documents).

/** One bar as the `"bars"` derived channel delivers it. */
interface DeliveredBar {
  label: string;
  current: number;
  max: number;
  color: string;
  mode: string;
}

/** One token's entry in the `"bars"` derived channel payload. */
interface DeliveredEntry {
  nameplate: string | null;
  nameplate_mode: string | null;
  bars: DeliveredBar[];
}

/** Records every `"bars"` derived frame the page receives and answers with the newest entry of its
 * only token.
 * @param page The page whose WebSocket frames to record; call before the world is entered.
 * @returns A function returning the newest delivered entry, or `null` before any token has one.
 */
function recordDeliveredBars(page: Page): () => DeliveredEntry | null {
  let latest: DeliveredEntry | null = null;
  page.on("websocket", (ws) => {
    ws.on("framereceived", ({ payload }) => {
      if (typeof payload !== "string" || !payload.includes('"scene_derived"')) return;
      const frame = JSON.parse(payload) as {
        type: string;
        channel: string;
        payload: { scenes: Record<string, { tokens: Record<string, DeliveredEntry> }> };
      };
      if (frame.type !== "scene_derived" || frame.channel !== "bars") return;
      const entries = Object.values(frame.payload.scenes).flatMap((scene) => Object.values(scene.tokens));
      latest = entries.length === 1 ? entries[0] : null;
    });
  });
  return () => latest;
}

// A 1×1 PNG used as token art (same fixture the stage/senses suites use).
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// In canvas-local pixels, inside the canvas: the docked actors panel narrows the canvas, so a point
// past its right edge lands on the panel instead.
const PLACE_AT = { x: 380, y: 460 };

test.use({ viewport: { width: 1600, height: 1000 } });

function stageHost(page: Page): Locator {
  return page.locator(".stage-host");
}

/** Toggle a launcher panel open or closed. */
async function togglePanel(page: Page, id: string): Promise<void> {
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId(`launcher-item-${id}`).click();
}

test("a GM authors actor bars and a per-token bar override that persist", async ({ page, account }) => {
  const worldName = `Bars World ${Date.now().toString(36)}`;
  const delivered = recordDeliveredBars(page);
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const snap = page.getByTestId("snap-toggle");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-pressed", "false");

  await togglePanel(page, "asset-browser:panel");
  await page.getByTestId("asset-upload-input").setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);
  await togglePanel(page, "asset-browser:panel");

  await togglePanel(page, "actors:panel");
  const actors = page.locator(".actors");
  await actors.getByLabel("Name", { exact: true }).fill("Brute");
  await actors.getByLabel("New independent copy on each placement").uncheck();
  await actors.getByTestId("visual-pick").click();
  const dialog = page.getByTestId("asset-pick-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("asset-tile").first().click();
  await dialog.getByTestId("pick-confirm").click();
  await expect(dialog).not.toBeVisible();

  // The create form's own bar editor: one bar, always shown, with the nameplate always shown.
  const form = actors.locator("form");
  await form.getByTestId("create-nameplate-mode").selectOption("always");
  await form.getByTestId("bar-add").click();
  await form.getByTestId("bar-current").fill("7");
  await form.getByTestId("bar-current").press("Tab");
  await form.getByTestId("bar-max").fill("10");
  await form.getByTestId("bar-max").press("Tab");
  await form.getByTestId("bar-mode").selectOption("always");
  await actors.getByRole("button", { name: "Create actor" }).click();

  // The row editor reads the stored values back.
  const row = actors.getByRole("listitem");
  await expect(row).toHaveCount(1);
  await expect(row.getByTestId("actor-nameplate-mode")).toHaveValue("always");
  await expect(row.getByTestId("bar-current")).toHaveValue("7");
  await expect(row.getByTestId("bar-max")).toHaveValue("10");
  await expect(row.getByTestId("bar-mode")).toHaveValue("always");

  // Place a linked token and select it; the per-token control appears and inherits by default.
  await row.getByRole("button", { name: "Brute" }).click();
  await page.getByTestId("tool-place").click();
  await clickScene(page, PLACE_AT);
  await expect(stageHost(page)).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
  await page.getByTestId("tool-select").click();
  await clickScene(page, PLACE_AT);
  const source = actors.getByTestId("token-bars-source");
  await expect(source).toHaveValue("inherit");

  // The delivered entry the drawing code reads: the nameplate and the one bar are shown always,
  // with the actor's values.
  await expect.poll(() => delivered()?.nameplate_mode, { timeout: 15_000 }).toBe("always");
  expect(delivered()?.nameplate).toBe("Brute");
  expect(delivered()?.bars.map((b) => [b.current, b.max, b.mode])).toEqual([[7, 10, "always"]]);

  // Switching to a custom list seeds it from the actor's; editing it leaves the actor untouched.
  await source.selectOption("custom");
  const tokenBars = actors.locator(".token-bars").getByTestId("bar-current");
  await expect(tokenBars).toHaveValue("7");
  await tokenBars.fill("3");
  await tokenBars.press("Tab");
  await expect(tokenBars).toHaveValue("3");
  await expect(row.getByTestId("bar-current")).toHaveValue("7");

  // The token's own list is what the drawing code now reads: its bar delivers 3, not the actor's 7.
  await expect.poll(() => delivered()?.bars.map((b) => b.current), { timeout: 15_000 }).toEqual([3]);
});
