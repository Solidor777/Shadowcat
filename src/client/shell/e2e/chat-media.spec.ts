import { test, expect, login } from "./fixtures";

// A 1×1 PNG, uploaded as an in-memory buffer.
const PNG_1X1 = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgAB" +
    "DQottAAAAABJRU5ErkJggg==",
  "base64",
);

test("the composer's image button is gated by the chat images setting, and a sent image renders in the card", async ({
  page,
  account,
}) => {
  await login(page, account.username, account.password);

  await expect(page.getByText("Your worlds")).toBeVisible();
  await page.getByLabel("New world name").fill("Chat Media World");
  await page.getByRole("button", { name: "Create world" }).click();

  // Chat panel docks by default; the "Insert an image" button is absent
  // until the GM turns the chat images setting on.
  await expect(page.getByTitle("Insert an image")).toHaveCount(0);

  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-game-settings:panel").click();
  await page.getByLabel("Images: asset links and server-fetched copies of linked images; never hotlinked", { exact: true }).check();

  await expect(page.getByTitle("Insert an image")).toBeVisible();

  // Upload an asset to insert.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "map.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Insert via the composer's image button: opens the pick overlay. A tile click only
  // SELECTS (`AssetGrid`'s `onclick` -> `select`); the pick is confirmed through
  // `PickConfirmBar`'s confirm button, the primary path every pick mode shares (double-click
  // is a single-select shortcut, and one a touch pointer cannot rely on).
  await page.getByTitle("Insert an image").click();
  const pickDialog = page.getByTestId("asset-pick-dialog");
  await expect(pickDialog).toBeVisible();
  await pickDialog.getByTestId("asset-tile").click();
  await pickDialog.getByTestId("pick-confirm").click();
  await expect(pickDialog).toHaveCount(0);

  // `exact`: the tool rail's "Send emote" button also matches a substring "Send".
  await page.getByRole("button", { name: "Send", exact: true }).click();

  const card = page.locator(".card").filter({ has: page.getByTestId("image-segment") });
  await expect(card).toHaveCount(1);
  const src = await card.getByTestId("image-segment").getAttribute("src");
  expect(src).toMatch(/\/api\/assets\//);
});
