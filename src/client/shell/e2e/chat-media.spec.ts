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
  await page.getByLabel("gameSettings.chat.images").check();

  await expect(page.getByTitle("Insert an image")).toBeVisible();

  // Upload an asset to insert.
  await page.getByTestId("launcher-trigger").click();
  await page.getByTestId("launcher-item-asset-browser:panel").click();
  await expect(page.getByTestId("asset-browser")).toBeVisible();
  await page
    .getByTestId("asset-upload-input")
    .setInputFiles({ name: "map.png", mimeType: "image/png", buffer: PNG_1X1 });
  await expect(page.getByTestId("asset-tile")).toHaveCount(1);

  // Insert via the composer's image button: opens the pick overlay in
  // single-select mode, where clicking the one tile confirms immediately.
  await page.getByTitle("Insert an image").click();
  await expect(page.getByTestId("asset-pick-dialog")).toBeVisible();
  await page.getByTestId("asset-pick-dialog").getByTestId("asset-tile").click();
  await expect(page.getByTestId("asset-pick-dialog")).toHaveCount(0);

  await page.getByRole("button", { name: "Send" }).click();

  const card = page.locator(".card").filter({ has: page.getByTestId("image-segment") });
  await expect(card).toHaveCount(1);
  const src = await card.getByTestId("image-segment").getAttribute("src");
  expect(src).toMatch(/\/api\/assets\//);
});
