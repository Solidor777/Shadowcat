import { test, expect } from "@playwright/test";

// A 1×1 PNG, uploaded as an in-memory buffer.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

test("upload an image, see the thumbnail, replace it, then delete it", async ({ page }) => {
  await page.goto("/");
  await page.getByLabel("Username").fill("ops");
  await page.getByLabel("Password").fill("pw-boot");
  await page.getByRole("button", { name: "Log in" }).click();

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Asset World");
  await page.getByRole("button", { name: "Create world" }).click();

  // In-world: the Assets panel is in the sidebar.
  await expect(page.getByRole("heading", { name: "Assets" })).toBeVisible();

  // Upload.
  await page
    .getByTestId("asset-upload")
    .setInputFiles({ name: "map.png", mimeType: "image/png", buffer: PNG_1X1 });
  const tile = page.getByTestId("asset-tile");
  await expect(tile).toHaveCount(1);

  // Replace (the tile persists; same UUID, new bytes).
  await tile.locator('input[type="file"]').setInputFiles({
    name: "map2.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Delete.
  await tile.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByTestId("asset-tile")).toHaveCount(0);
});
