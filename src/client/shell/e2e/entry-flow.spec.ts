import { test, expect } from "@playwright/test";

// Drives the real served binary: the SPA boots, sees an initialized server, and
// the user logs in (admin seeded as ops/pw-boot), creates a world, and reaches the
// in-world table shell.
test("login → world-select → enter table (served by the binary)", async ({ page }) => {
  await page.goto("/");

  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Smoke World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Entering a world mounts the Pixi stage canvas.
  await expect(page.getByTestId("stage-canvas")).toBeVisible();
});
