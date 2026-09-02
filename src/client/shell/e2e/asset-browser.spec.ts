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

test("a multi-chunk upload lands, takes a tag, and is found by the tag filter", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Chunk World");
  await page.getByRole("button", { name: "Create world" }).click();

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();

  // >1 chunk: 9 MiB against the fixed 8 MiB chunk size.
  const big = Buffer.alloc(9 * 1024 * 1024, 7);
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "big.bin", mimeType: "application/octet-stream", buffer: big });
  // The queue shows progress, then the listing refreshes with the new tile.
  await expect(page.getByTestId("upload-queue")).toBeVisible();
  await expect(page.getByTestId("asset-tile")).toHaveCount(1, { timeout: 60_000 });

  // Tag it in the preview pane.
  await page.getByTestId("asset-tile").click();
  await page.getByTestId("preview-tag-input").fill("bulkmap");
  await page.getByTestId("preview-tag-input").press("Enter");
  await expect(page.getByTestId("preview-tag-remove-bulkmap")).toBeVisible();

  // Find it by the tag filter (a chip in the filter bar).
  await page.getByTestId("filter-tag-input").fill("bulkmap");
  await page.getByTestId("filter-tag-input").press("Enter");
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);
  // A non-matching tag empties the grid — the filter is real, not a no-op.
  await page.getByTestId("filter-tag-remove-bulkmap").click();
  await page.getByTestId("filter-tag-input").fill("nomatch");
  await page.getByTestId("filter-tag-input").press("Enter");
  await expect(page.getByTestId("asset-browser-empty")).toBeVisible();
});

test("a folder moves under another via the accessible move control", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);
  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Folder World");
  await page.getByRole("button", { name: "Create world" }).click();

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();

  // Two root folders.
  await page.getByTestId("folder-create-name").fill("alpha");
  await page.getByTestId("folder-create-name").press("Enter");
  await page.getByTestId("folder-create-name").fill("beta");
  await page.getByTestId("folder-create-name").press("Enter");
  const betaRow = page.locator(".row", { hasText: "beta" });
  await expect(betaRow).toHaveCount(1);
  await expect(betaRow).toHaveAttribute("style", /padding-left: 0rem/);

  // Move beta under alpha via the Move-to picker (the Move document op,
  // end to end: intent -> server validity -> broadcast -> store -> tree).
  await betaRow.getByTitle("Move folder").click();
  await page.locator(".move-picker button", { hasText: "alpha" }).click();
  await expect(page.locator(".row", { hasText: "beta" })).toHaveAttribute(
    "style",
    /padding-left: 0.75rem/,
  );
});
