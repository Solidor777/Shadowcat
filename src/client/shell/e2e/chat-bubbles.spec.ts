import { test, expect, login } from "./fixtures";
import { clickScene, sceneCenter } from "./stage-gestures";

// A 1x1 PNG used as token art.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// A chat message sent "as" a selected token raises a speech bubble over that token in the stage
// overlay layer, and the bubble removes itself after its display time. The bubble stays up for
// several seconds, so a retried visibility assertion cannot straddle its whole lifetime; the
// disappearance is asserted with a timeout longer than that lifetime.
test("a message sent as a placed token shows a bubble over the token that later disappears", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Bubble World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  await page.getByTestId("tool-place").click();
  const pick = page.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();

  const center = await sceneCenter(page);
  await clickScene(page, center);
  await expect(host).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

  // No bubble before anyone speaks.
  await expect(page.getByTestId("chat-bubble")).toHaveCount(0);

  // Select the token, mark the next message as spoken by it, and send.
  await page.getByTestId("tool-select").click();
  await clickScene(page, center);
  await page.getByTestId("speak-as-token").click();
  const text = `bubble-${Date.now().toString(36)}`;
  await page.locator("#chat-composer-input").fill(text);
  await page.locator("#chat-composer-input").press("Enter");

  const bubble = page.getByTestId("chat-bubble");
  await expect(bubble).toHaveCount(1, { timeout: 15_000 });
  await expect(bubble).toContainText(text);

  // The bubble sits above the token: its box ends above the canvas centre where the token stands.
  const canvasBox = await page.getByTestId("stage-canvas").boundingBox();
  const bubbleBox = await bubble.boundingBox();
  expect(canvasBox).not.toBeNull();
  expect(bubbleBox).not.toBeNull();
  expect(bubbleBox!.y + bubbleBox!.height).toBeLessThan(canvasBox!.y + center.y);

  // It removes itself after its display time.
  await expect(bubble).toHaveCount(0, { timeout: 20_000 });
});
