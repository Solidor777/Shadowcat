import { test, expect, login, type WorkerAccount } from "./fixtures";

async function enterFreshWorld(
  page: import("@playwright/test").Page,
  name: string,
  account: WorkerAccount,
): Promise<void> {
  await login(page, account.username, account.password);
  await page.getByLabel("New world name").fill(name);
  await page.getByRole("button", { name: "Create world" }).click();
  await expect(page.locator(".stage-host")).toHaveAttribute(
    "data-render-ready",
    "true",
    {
      timeout: 30_000,
    },
  );
}

// Theme application writes every token as an inline custom property on
// `document.documentElement` (`ThemeController.applyTo`), so a switch is
// observable straight off the element's inline style. `--surface-base` is
// #1f1f2c under the slate-dark default and #e4e6f0 under slate-light.
function surfaceBase(page: import("@playwright/test").Page): Promise<string> {
  return page.evaluate(() =>
    document.documentElement.style.getPropertyValue("--surface-base"),
  );
}

test("a theme switch applies immediately, persists to ui-state, and survives a reload", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Theme Persistence World", account);

  // The default theme is applied inline even without a saved preference.
  await expect.poll(() => surfaceBase(page)).toBe("#1f1f2c");

  await page.getByTestId("topbar-settings").click();
  const themeSelect = page.getByLabel("Theme");
  await expect(themeSelect).toBeVisible();

  // The persist is a leading-edge fire-and-forget PUT (`persist`); register
  // the wait ahead of the selection that triggers it, then await the response
  // prior to reloading, or the navigation can abort the in-flight PUT. The
  // payload clause pins the wait to the PUT carrying `global.theme` — the
  // world entry itself persists other global leaves (`lastWorld`) first.
  const persistResponse = page.waitForResponse((r) => {
    if (
      !/\/api\/me\/ui-state$/.test(r.url()) ||
      r.request().method() !== "PUT" ||
      !r.ok()
    ) {
      return false;
    }
    try {
      const body = r.request().postDataJSON() as {
        global?: { theme?: { active?: string } };
      };
      return body?.global?.theme?.active === "slate-light";
    } catch {
      return false;
    }
  });
  await themeSelect.selectOption("slate-light");
  await expect.poll(() => surfaceBase(page)).toBe("#e4e6f0");
  await persistResponse;

  // A full reload re-runs boot from scratch: the ui-state fetch must restore
  // the light theme (inline styles present again after load).
  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute(
    "data-render-ready",
    "true",
    {
      timeout: 30_000,
    },
  );
  await expect.poll(() => surfaceBase(page)).toBe("#e4e6f0");
});
