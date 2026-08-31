import { test, expect, login } from "./fixtures";

// A 1×1 PNG, uploaded as an in-memory buffer.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

test("upload an image, see the thumbnail, replace it, then delete it", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Asset World");
  await page.getByRole("button", { name: "Create world" }).click();

  // In-world: the browser panel starts launcher-closed; open it from the
  // topbar launcher.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();

  // Upload.
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "map.png", mimeType: "image/png", buffer: PNG_1X1 });
  const tile = page.getByTestId("asset-tile");
  await expect(tile).toHaveCount(1);

  // Replace via the preview pane (the tile persists; same UUID, new bytes).
  await tile.click();
  await page.getByTestId("preview-replace").setInputFiles({
    name: "map2.png",
    mimeType: "image/png",
    buffer: PNG_1X1,
  });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Delete via the preview pane, with its confirm step.
  await page.getByTestId("asset-tile").click();
  await page.getByTestId("preview-delete").click();
  await page.getByTestId("preview-delete-confirm").click();
  await expect(page.getByTestId("asset-tile")).toHaveCount(0);
});
