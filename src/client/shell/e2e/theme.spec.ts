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

// Custom themes are user-authored data: the editor's draft previews live
// without persisting, save persists through the ui-state machinery, and
// deleting the active theme falls back to the default.
test("a custom theme is authored, persisted across reload, and deleted with fallback", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Custom Theme World", account);
  await page.getByTestId("topbar-settings").click();

  await page.getByRole("button", { name: "New custom theme" }).click();
  await page.getByLabel("Theme name").fill("My Theme");

  // Live preview: editing a token applies it immediately, before any save.
  const accentRow = page.locator(".row", {
    has: page.getByText("--accent", { exact: true }),
  });
  await accentRow.locator('input[type="color"]').fill("#ff0000");
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--accent")),
    )
    .toBe("#ff0000");

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
      return body?.global?.theme?.active?.startsWith("custom:") === true;
    } catch {
      return false;
    }
  });
  await page.getByRole("button", { name: "Save theme" }).click();
  await persistResponse;

  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute(
    "data-render-ready",
    "true",
    { timeout: 30_000 },
  );
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--accent")),
    )
    .toBe("#ff0000");

  // Delete the active custom theme: the confirm is accepted, and the theme
  // falls back to the default's accent. The settings panel was open across
  // the reload and the panel layout is persisted, so it may already be
  // restored open — the topbar button toggles, so click it only when the
  // panel is not showing.
  page.once("dialog", (d) => d.accept());
  const newThemeButton = page.getByRole("button", { name: "New custom theme" });
  if (!(await newThemeButton.isVisible())) {
    await page.getByTestId("topbar-settings").click();
  }
  await expect(newThemeButton).toBeVisible();
  const row = page.locator(".custom-theme-row", { hasText: "My Theme" });
  await row.getByRole("button", { name: "Delete" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.style.getPropertyValue("--accent")),
    )
    .toBe("#2d6ee8");
  await expect(page.getByLabel("Theme")).toHaveValue("slate-dark");
});

// Theme isolation end to end in a real browser: with a non-default theme
// active, an element inside the isolation class must compute the DEFAULT
// theme's token values while an element outside computes the active theme's.
// The contribution-wrapping half of the chain (who gets the class) is
// unit-tested in Surface/PanelHost; this proves the mechanism those wrappers
// rely on — the per-document sheet plus the cascade.
test("an isolated subtree keeps engine defaults under a non-default theme", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Theme Isolation World", account);
  await page.getByTestId("topbar-settings").click();
  await page.getByLabel("Theme").selectOption("slate-light");
  await expect.poll(() => surfaceBase(page)).toBe("#e4e6f0");

  const probe = await page.evaluate(() => {
    const host = document.createElement("div");
    host.innerHTML =
      '<div class="sc-theme-isolate"><div id="iso-probe" style="background: var(--surface-base)"></div></div>' +
      '<div id="host-probe" style="background: var(--surface-base)"></div>';
    document.body.appendChild(host);
    const read = (id: string) => getComputedStyle(document.getElementById(id)!).backgroundColor;
    const result = { isolated: read("iso-probe"), host: read("host-probe") };
    host.remove();
    return result;
  });
  // Computed colors come back as rgb(): the slate-light surface vs the
  // slate-dark default the isolated subtree must keep.
  expect(probe.host).toBe("rgb(228, 230, 240)"); // #e4e6f0
  expect(probe.isolated).toBe("rgb(31, 31, 44)"); // #1f1f2c
});
