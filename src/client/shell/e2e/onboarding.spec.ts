import { test, expect, login, createAccount, openPanel, newE2EContext, DUAL_SESSION_TIMEOUT_MS } from "./fixtures";
import type { Browser, Page } from "@playwright/test";

// First-use onboarding, against the `onboarding` project's server: it has NO seeded admin and the
// tour offers are on (every other e2e server runs with them off). The tests run in order on one
// server: the first creates the admin through the setup screen and reads the Welcome screen that
// follows, and the rest sign in as that admin to mint the fresh accounts they exercise, because a
// tour's done-state is stored per account and a reused account would start with it already set.

const ADMIN_NAME = "onboarding-admin";
const ADMIN_PASSWORD = "pw-onboarding-admin";
const MEMBER_PASSWORD = "pw-onboarding-member";

const VIEWPORT = { width: 1600, height: 1000 };
test.use({ viewport: VIEWPORT });

test.describe.configure({ mode: "serial" });

function stageHost(page: Page) {
  return page.locator(".stage-host");
}

/** Waits for the page to be inside a world's stage, entering the named world from the world list
 * when the page is showing that list instead (a reload lands on either).
 * @param page The page to drive.
 * @param worldName The world to enter when the list shows.
 */
async function reachStage(page: Page, worldName: string): Promise<void> {
  const listed = page.getByRole("button", { name: new RegExp(worldName) });
  await stageHost(page).or(listed).first().waitFor({ timeout: 30_000 });
  if (!(await stageHost(page).isVisible())) await listed.click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
}

/** Creates a world named `worldName` from the signed-in world list and enters it.
 * @param page The page to drive.
 * @param worldName The new world's name.
 */
async function createAndEnterWorld(page: Page, worldName: string): Promise<void> {
  await page.getByLabel("New world name").fill(worldName);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(stageHost(page)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });
}

/** Has the admin create a plain account, in a context of its own that is closed afterwards.
 * @param browser The browser to mint the admin's context from.
 * @param baseURL The onboarding server's origin.
 * @param label A short label woven into the account name.
 * @returns The new account's username.
 */
async function mintAccount(browser: Browser, baseURL: string | undefined, label: string): Promise<string> {
  const username = `${label}-${Date.now().toString(36)}`;
  const ctx = await newE2EContext(browser, { baseURL, viewport: VIEWPORT });
  try {
    const admin = await ctx.newPage();
    await login(admin, ADMIN_NAME, ADMIN_PASSWORD);
    await createAndEnterWorld(admin, `Mint World ${username}`);
    await openPanel(admin, "settings:panel");
    await createAccount(admin, username, MEMBER_PASSWORD);
  } finally {
    await ctx.close();
  }
  return username;
}

test("setup creates the first admin, the Welcome screen follows, and the first world offers a tour that Don't ask again silences", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByLabel("Username").fill(ADMIN_NAME);
  await page.getByLabel("Password").fill(ADMIN_PASSWORD);
  await page.getByRole("button", { name: "Create admin" }).click();

  // The Welcome screen shows between setup and login, with the hosting guide link.
  const welcome = page.getByTestId("entry-welcome");
  await expect(welcome).toBeVisible();
  await expect(welcome.getByRole("link", { name: "Read the hosting guide" })).toHaveAttribute("href", /hosting\.md$/);
  await welcome.getByRole("button", { name: "Continue" }).click();

  // Continue lands on the login form; the new admin signs in.
  await expect(welcome).toHaveCount(0);
  await page.getByLabel("Username").fill(ADMIN_NAME);
  await page.getByLabel("Password").fill(ADMIN_PASSWORD);
  await page.getByRole("button", { name: "Log in" }).click();
  const worldName = `Welcome World ${Date.now().toString(36)}`;
  await createAndEnterWorld(page, worldName);

  // A first-time GM in a fresh world is offered a tour; "Don't ask again" closes it for good.
  const offer = page.getByTestId("tour-offer");
  await expect(offer).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("tour-offer-never").click();
  await expect(offer).toHaveCount(0);

  // Entering again must not offer one (the switch is stored on the account, not the session).
  await page.reload();
  await reachStage(page, worldName);
  // The positive control above proves an offer arrives within 15 s of a stage being ready when the
  // account has not switched them off, so a 3 s quiet period here is a real absence.
  await page.waitForTimeout(3_000);
  await expect(offer).toHaveCount(0);
});

test("a GM's offer answers Not now by returning next session, and Start walks a tour whose finish moves the offer on", async ({
  page,
  browser,
}) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const baseURL = test.info().project.use.baseURL;
  const gmName = await mintAccount(browser, baseURL, "gm");
  const worldName = `Offer World ${Date.now().toString(36)}`;

  await login(page, gmName, MEMBER_PASSWORD);
  await createAndEnterWorld(page, worldName);

  // Not now closes the offer without recording anything, so the next session offers again.
  const offer = page.getByTestId("tour-offer");
  await expect(offer).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("tour-offer-later").click();
  await expect(offer).toHaveCount(0);
  await page.reload();
  await reachStage(page, worldName);
  await expect(offer).toBeVisible({ timeout: 15_000 });

  // Start opens the first step; the tour is walked to its last step and finished.
  await page.getByTestId("tour-offer-start").click();
  await expect(offer).toHaveCount(0);
  const card = page.getByTestId("tour-card");
  await expect(card).toBeVisible();
  const firstTitle = await card.getByRole("heading").innerText();
  const total = Number(/of (\d+)/.exec(await page.getByTestId("tour-count").innerText())?.[1]);
  expect(total).toBeGreaterThan(0);
  await expect(page.getByTestId("tour-count")).toContainText(`Step 1 of ${total}`);
  for (let step = 1; step < total; step++) {
    await page.getByTestId("tour-next").click();
    await expect(page.getByTestId("tour-count")).toContainText(`Step ${step + 1} of ${total}`);
  }
  await expect(page.getByTestId("tour-next")).toHaveText("Done");
  await page.getByTestId("tour-next").click();
  await expect(page.getByTestId("tour-overlay")).toHaveCount(0);

  // The finished tour is recorded: the next session offers a different one.
  await page.reload();
  await reachStage(page, worldName);
  await expect(offer).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("tour-offer-start").click();
  await expect(card).toBeVisible();
  expect(await card.getByRole("heading").innerText()).not.toBe(firstTitle);
  await page.getByTestId("tour-skip").click();
  await expect(page.getByTestId("tour-overlay")).toHaveCount(0);
});

test("a GM replays the actors tour from the settings panel", async ({ page, browser }) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const gmName = await mintAccount(browser, test.info().project.use.baseURL, "actors");
  await login(page, gmName, MEMBER_PASSWORD);
  await createAndEnterWorld(page, `Actors Tour World ${Date.now().toString(36)}`);

  // The offer's toast is answered first so it never covers the panels the tour points at.
  await expect(page.getByTestId("tour-offer")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("tour-offer-later").click();

  await openPanel(page, "actors:panel");
  await openPanel(page, "settings:panel");
  await page.getByTestId("tour-replay-actors.intro").click();
  const card = page.getByTestId("tour-card");
  await expect(card).toContainText("The actors panel");
  await page.getByTestId("tour-next").click();
  await expect(card).toContainText("Search");
  await page.getByTestId("tour-next").click();
  await expect(card).toContainText("Actor list");
  // The quick-edit step needs a selected token, so it is skipped and the list step is the last.
  await expect(page.getByTestId("tour-next")).toHaveText("Done");
  await page.getByTestId("tour-next").click();
  await expect(page.getByTestId("tour-overlay")).toHaveCount(0);
});

test("an invited player is offered a tour and replays the chat tour from settings", async ({ page, browser }) => {
  test.setTimeout(DUAL_SESSION_TIMEOUT_MS);
  const baseURL = test.info().project.use.baseURL;
  const gmName = await mintAccount(browser, baseURL, "chatgm");
  const playerName = await mintAccount(browser, baseURL, "chatpl");
  const worldName = `Chat Tour World ${Date.now().toString(36)}`;

  // The GM creates the world and an invite for the player.
  const gm = page;
  await login(gm, gmName, MEMBER_PASSWORD);
  await createAndEnterWorld(gm, worldName);
  await openPanel(gm, "settings:panel");
  await gm.getByLabel("World role").selectOption("player");
  await gm.getByRole("button", { name: "Create invite" }).click();
  const code = await gm.getByLabel("Invite code").inputValue();
  expect(code.length).toBeGreaterThan(0);

  const playerCtx = await newE2EContext(browser, { baseURL, viewport: VIEWPORT });
  try {
    const player = await playerCtx.newPage();
    await login(player, playerName, MEMBER_PASSWORD);
    await player.getByLabel("Invite code").fill(code);
    await player.getByRole("button", { name: "Join with an invite code" }).click();
    await expect(stageHost(player)).toHaveAttribute("data-render-ready", "true", { timeout: 30_000 });

    // A first-time player is offered a tour too.
    const offer = player.getByTestId("tour-offer");
    await expect(offer).toBeVisible({ timeout: 15_000 });
    await player.getByTestId("tour-offer-later").click();

    // The chat tour is in the player's replay list and starts on the chat panel's own step.
    await openPanel(player, "settings:panel");
    await player.getByTestId("tour-replay-chat.intro").click();
    const card = player.getByTestId("tour-card");
    await expect(card).toContainText("The chat panel");
    await player.getByTestId("tour-next").click();
    await expect(card).toContainText("Channels");
    await player.getByTestId("tour-skip").click();
    await expect(player.getByTestId("tour-overlay")).toHaveCount(0);
  } finally {
    await playerCtx.close();
  }
});
