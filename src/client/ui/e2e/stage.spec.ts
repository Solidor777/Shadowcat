import { test, expect } from "@playwright/test";

// A 1×1 PNG used as token art.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

// Drives the served binary: after entering a world the Pixi canvas mounts, the
// engine reaches first-frame readiness, accepts a pan gesture, and tears down on
// leave. Real WebGL via headless chromium (SwiftShader).
test("stage canvas mounts, renders, and tears down on leave", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Render World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  const canvas = page.getByTestId("stage-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(0);
  expect(box?.height ?? 0).toBeGreaterThan(0);

  // A pan gesture must not throw (pointer events drive the camera).
  await canvas.hover();
  await page.mouse.down();
  await page.mouse.move((box!.x) + 50, (box!.y) + 50);
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-render-ready", "true");

  // Leave-world tears the canvas down.
  await page.getByRole("button", { name: /leave world/i }).click();
  await expect(page.getByTestId("stage-canvas")).toHaveCount(0);
});

test("place a token via the tool rail, then drag it", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Token World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // Upload an image asset (the token art).
  await page
    .getByTestId("asset-upload")
    .setInputFiles({ name: "tok.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Activate the place tool and pick the asset in the rail's picker.
  await page.getByTestId("tool-place").click();
  const pick = page.getByTestId("picker-asset").first();
  await expect(pick).toBeVisible({ timeout: 10_000 });
  await pick.click();

  // Click the canvas → a token document is created (optimistic) and rendered.
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  await expect(host).toHaveAttribute("data-token-count", "1", { timeout: 15_000 });

  // Drag the token with the select/move tool: it must not throw and the token persists.
  await page.getByTestId("tool-select").click();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2 + 40, { steps: 4 });
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-token-count", "1");
});

test("draw a freehand stroke via the tool rail; the drawing renders", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Draw World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await page.getByTestId("tool-draw").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  // Drag a freehand path across the canvas.
  await page.mouse.move(box.x + box.width / 2 - 40, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 - 30, { steps: 3 });
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2, { steps: 3 });
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-shape-count", "1", { timeout: 15_000 });
});

test("ping a location via the tool rail; the relayed ping renders", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Ping World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await page.getByTestId("tool-ping").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await canvas.click({ position: { x: box.width / 2, y: box.height / 2 } });
  // The server relays the ping back to the sender → Stage's onPing sets data-last-ping.
  await expect(host).toHaveAttribute("data-last-ping", /.+/, { timeout: 15_000 });
});

test("draw a wall via the tool rail; the wall renders", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Wall World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  await page.getByTestId("tool-wall").click();
  const canvas = page.getByTestId("stage-canvas");
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2 - 60, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2 + 20, { steps: 3 });
  await page.mouse.up();
  await expect(host).toHaveAttribute("data-wall-count", "1", { timeout: 15_000 });
});

test("the identity SceneDerived spike reaches the mask slot", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Vision Spike World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Entering a world subscribes to the "identity" channel; the server pushes an
  // initial frame, the engine applies it (watermark-gated) and sets the signal.
  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
  await expect(host).toHaveAttribute("data-scene-derived", "1", { timeout: 30_000 });
});
