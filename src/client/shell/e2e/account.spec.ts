import { test, expect, login, createAccount, newE2EContext } from "./fixtures";

// A password change revokes every session for the account, so this test changes the
// password of a throwaway account it creates, never the worker account the rest of the suite shares.
test("changing the password ends the session; only the new password logs in afterwards", async ({
  page,
  browser,
  account,
}) => {
  const suffix = Date.now().toString(36);
  const username = `e2e-pw-${suffix}`;
  const oldPassword = "pw-old-e2e";
  const newPassword = "pw-new-e2e-changed";

  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(`Password World ${suffix}`);
  await page.getByRole("button", { name: "Create world" }).click();
  await page.getByTestId("topbar-settings").click();
  await createAccount(page, username, oldPassword);

  const userContext = await newE2EContext(browser);
  try {
    const userPage = await userContext.newPage();
    await login(userPage, username, oldPassword);
    await userPage.getByLabel("New world name").fill(`Own World ${suffix}`);
    await userPage.getByRole("button", { name: "Create world" }).click();
    await userPage.getByTestId("topbar-settings").click();

    // A wrong current password is refused and the session survives.
    await userPage.getByLabel("Current password").fill("not-the-password");
    await userPage.getByLabel("New password", { exact: true }).fill(newPassword);
    await userPage.getByLabel("Confirm new password").fill(newPassword);
    await userPage.getByRole("button", { name: "Change password" }).click();
    await expect(userPage.getByText("The current password is incorrect.")).toBeVisible();

    // The correct current password changes it and returns to the login screen.
    await userPage.getByLabel("Current password").fill(oldPassword);
    await userPage.getByRole("button", { name: "Change password" }).click();
    await expect(userPage.getByLabel("Username")).toBeVisible();

    // The old password no longer authenticates; the new one does.
    const oldAttempt = await userPage.request.post("/api/login", {
      data: { username, password: oldPassword },
    });
    expect(oldAttempt.status()).toBe(401);
    await login(userPage, username, newPassword);
    await expect
      .poll(async () => (await userPage.request.get("/api/me")).status())
      .toBe(200);
  } finally {
    await userContext.close();
  }
});
