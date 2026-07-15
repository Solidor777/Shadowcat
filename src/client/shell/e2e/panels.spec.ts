import { test, expect } from "@playwright/test";

async function enterFreshWorld(page: import("@playwright/test").Page, name: string): Promise<void> {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });
}

// Non-chat panels start launcher-closed (absent from the layout), not minimized
// chips. Assets opens from the topbar launcher, docks right on first toggle, and
// survives a full reload (the persisted-source path).
test("a panel opened from the launcher docks and survives a full page reload", async ({ page }) => {
  await enterFreshWorld(page, "Launcher Persistence World");

  const uploadInput = page.getByTestId("asset-upload");
  // Launcher-closed: assets content is not mounted-visible, and there is no chip.
  await expect(uploadInput).not.toBeVisible();
  await expect(page.getByTestId("chip-assets:panel")).toHaveCount(0);

  // Open it from the topbar launcher menu.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await expect(uploadInput).toBeVisible();

  // Full reload re-runs module registration/activation from scratch.
  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", {
    timeout: 30_000,
  });

  // The docked assets panel survives instead of reverting to launcher-closed.
  await expect(page.getByTestId("asset-upload")).toBeVisible();
});

// Toggling the same launcher item again minimizes the (now docked) panel — the
// dock-chip metaphor: it hides the body and drops a statusbar restore chip.
test("re-toggling a launcher item minimizes the open panel to a dock chip", async ({ page }) => {
  await enterFreshWorld(page, "Launcher Toggle World");

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await expect(page.getByTestId("asset-upload")).toBeVisible();

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-assets:panel").click();
  await expect(page.getByTestId("asset-upload")).not.toBeVisible();
  await expect(page.getByTestId("chip-assets:panel")).toHaveCount(1);
});

// Single breakpoint axis (48rem): a narrow viewport puts the layout into compact
// (grid switch + bottom tool strip); a wide viewport does not. Directly pins the
// ui-kit sizeClass axis.
test("the layout grid keys compact/expanded off the single 48rem axis", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await enterFreshWorld(page, "Compact Axis World");

  await expect(page.locator(".layout")).not.toHaveClass(/\bcompact\b/);

  await page.setViewportSize({ width: 500, height: 900 });
  await expect(page.locator(".layout")).toHaveClass(/\bcompact\b/);

  // The launcher remains reachable on compact (icon-only trigger).
  await expect(page.getByTestId("launcher-trigger")).toBeVisible();

  // The tool rail's compact path is untestable under jsdom, so this block is its
  // only automated coverage. Requires a GM session (enterFreshWorld creates the
  // world as GM, so the rail renders). The rail must carry the compact class and
  // its tool buttons must remain reachable in the horizontal strip.
  await expect(page.locator(".tool-rail")).toHaveClass(/\bcompact\b/);
  const firstTool = page.locator(".tool-rail .tool").first();
  await firstTool.scrollIntoViewIfNeeded();
  await expect(firstTool).toBeVisible();
});
