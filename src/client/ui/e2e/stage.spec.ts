import { test, expect } from "@playwright/test";

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
