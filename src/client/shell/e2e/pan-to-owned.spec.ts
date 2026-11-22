import { test, expect, login } from "./fixtures";
import { clickScene, dragScene, sceneCenter } from "./stage-gestures";

// A 1x1 PNG used as token art.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

/** Parses a `data-camera-center` / token-position pair `"x,y"` into numbers. */
function parsePoint(raw: string | null): { x: number; y: number } {
  expect(raw, "the stage host must publish the point").not.toBeNull();
  const [x, y] = raw!.split(",").map(Number);
  return { x: x!, y: y! };
}

// The pan-to-owned button returns the camera to the user's own token after the camera was
// dragged away. The camera centre is read from the stage host's `data-camera-center` signal (the
// scene point at the viewport centre) and compared against the token's committed centre from
// `data-token-positions`.
test("pan-to-owned returns the camera centre to the owned token after a camera drag", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill("Pan World");
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

  // No owned token yet: the button is disabled.
  await expect(page.getByTestId("pan-to-owned")).toBeDisabled();

  const center = await sceneCenter(page);
  await clickScene(page, center);
  await expect(host).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });
  // A placed token has no owner, so the button stays disabled until the GM assigns the token to
  // themself: select it, then set the override in the Actors panel's token-owner control.
  await expect(page.getByTestId("pan-to-owned")).toBeDisabled();
  await page.getByTestId("tool-select").click();
  await clickScene(page, center);
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-actors:panel").click();
  await page.locator(".actors").getByLabel("Token owner").selectOption({ label: account.username });
  await expect(page.getByTestId("pan-to-owned")).toBeEnabled();

  const positions = (await host.getAttribute("data-token-positions")) ?? "";
  const token = parsePoint(positions.split(":")[1] ?? null);

  // Drag the camera away from the token; the drag starts on empty canvas far from the token.
  await page.getByTestId("tool-select").click();
  const before = parsePoint(await host.getAttribute("data-camera-center"));
  await dragScene(page, { x: center.x + 300, y: center.y + 200 }, { x: center.x + 100, y: center.y + 100 }, 5);
  await expect
    .poll(async () => parsePoint(await host.getAttribute("data-camera-center")).x)
    .not.toBeCloseTo(before.x, 0);
  const away = parsePoint(await host.getAttribute("data-camera-center"));
  expect(Math.hypot(away.x - token.x, away.y - token.y)).toBeGreaterThan(50);

  await page.getByTestId("pan-to-owned").click();
  await expect
    .poll(async () => {
      const c = parsePoint(await host.getAttribute("data-camera-center"));
      return Math.hypot(c.x - token.x, c.y - token.y);
    })
    .toBeLessThan(1);
});
