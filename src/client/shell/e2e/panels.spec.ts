import { test, expect } from "@playwright/test";

// B4: every panel-contract module `requires` PANEL_CONTRACT, so `panels` topologically
// activates BEFORE any of them — `PanelsController` is routinely constructed against an
// empty (or partial) registry. Without the persisted-source fix, each late-registering
// panel's `syncRegistrations` catch-up default-placed it and re-persisted a defaults-shaped
// tree, silently discarding whatever the user had actually saved on every reload. This is
// the end-to-end reproduction: dock a panel that starts minimized, reload the real served
// app, and assert the dock survived instead of reverting to the default.
test("a restored (docked) panel's layout survives a full page reload", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();
  await page.getByLabel("New world name").fill("Panel Persistence World");
  await page.getByRole("button", { name: "Create world" }).click();

  const host = page.locator(".stage-host");
  await expect(host).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // Assets starts minimized (M12a interim default) — its content is mounted but hidden.
  const uploadInput = page.getByTestId("asset-upload");
  await expect(uploadInput).not.toBeVisible();

  // Restore it via the statusbar's dock-chip strip — docks it into "right" (`applyOp`'s
  // `restore` op), a non-default location distinct from its `{kind:"minimized"}` default.
  await page.getByTestId("chip-assets:panel").click();
  await expect(uploadInput).toBeVisible();
  await expect(page.getByTestId("chip-assets:panel")).toHaveCount(0);

  // Full reload: a fresh page load re-runs module registration/activation from scratch,
  // reproducing the exact boot-race window this fix addresses.
  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

  // Without the fix, this reload would have reset the layout to defaults (assets minimized
  // again). With the fix, the docked customization survives.
  await expect(page.getByTestId("asset-upload")).toBeVisible();
  await expect(page.getByTestId("chip-assets:panel")).toHaveCount(0);
});
