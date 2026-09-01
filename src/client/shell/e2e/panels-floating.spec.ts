import { test, expect, login, type WorkerAccount } from "./fixtures";
import type { Page } from "@playwright/test";

async function enterFreshWorld(
  page: Page,
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

/** Opens the chat tab's per-tab command menu (the `⋮` button the tab renderer
 * appends) and clicks one command item. */
async function chatTabMenu(page: Page, commandTestId: string): Promise<void> {
  const chatTab = page.locator(".dv-tab", { hasText: "Chat" }).first();
  await chatTab.locator(".sc-tab-menu-btn").click();
  await page.getByTestId(commandTestId).click();
}

/** A ui-state PUT whose `panelLayout` blob satisfies `match`. Every persist
 * body looks alike at the method+URL level, so only a payload clause pins the
 * wait to the PUT that carries the gesture being tested — see the same pattern
 * in the launcher-persistence spec. */
function layoutPut(
  page: Page,
  match: (layout: unknown) => boolean,
): Promise<import("@playwright/test").Response> {
  return page.waitForResponse((r) => {
    if (
      !/\/api\/me\/ui-state$/.test(r.url()) ||
      r.request().method() !== "PUT" ||
      !r.ok()
    ) {
      return false;
    }
    try {
      const body = r.request().postDataJSON() as {
        worlds?: Record<string, { panelLayout?: unknown }>;
      };
      for (const w of Object.values(body?.worlds ?? {})) {
        if (w.panelLayout && match(w.panelLayout)) return true;
      }
      return false;
    } catch {
      return false;
    }
  });
}

interface FloatingEntry {
  id: string;
  rect: { x: number; y: number; w: number; h: number };
}

/** Extracts the chat panel's floating entry from a persisted layout blob. */
function chatFloating(layout: unknown): FloatingEntry | undefined {
  const expanded = (layout as { expanded?: { floating?: FloatingEntry[] } })
    ?.expanded;
  return expanded?.floating?.find((f) => f.id === "chat:panel");
}

// Keyboard resize is a command path over the same `resizeFloating` op a
// pointer drag emits: the focused floating dialog turns Ctrl+ArrowRight into
// an 8px width step, the op round-trips through the tree, and the persisted
// layout carries the new size across a reload.
test("keyboard resize of a floating panel persists across a reload", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Floating Keyboard World", account);


  const floatPut = layoutPut(page, (l) => chatFloating(l) !== undefined);
  await chatTabMenu(page, "panel-menu-float");
  const dialog = page.locator('[role="dialog"]');
  await expect(dialog).toBeVisible();
  const floatBody = await floatPut;
  const initial = (() => {
    const body = floatBody.request().postDataJSON() as {
      worlds: Record<string, { panelLayout: unknown }>;
    };
    for (const w of Object.values(body.worlds)) {
      const f = chatFloating(w.panelLayout);
      if (f) return f;
    }
    throw new Error("float PUT carried no chat floating entry");
  })();

  // The dialog wrapper is focused on creation; Ctrl+ArrowRight widens by one
  // 8px step. Wait for the focus to actually land — the menu's own
  // focus-return runs in the same gesture and the dialog's focus wins only at
  // the end of it.
  await expect(dialog).toBeFocused();
  const resizePut = layoutPut(
    page,
    (l) => chatFloating(l)?.rect.w === initial.rect.w + 8,
  );
  await page.keyboard.press("Control+ArrowRight");
  await resizePut;
  await expect
    .poll(async () => (await dialog.boundingBox())?.width)
    .toBe(initial.rect.w + 8);

  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute(
    "data-render-ready",
    "true",
    {
      timeout: 30_000,
    },
  );
  const dialogAfter = page.locator('[role="dialog"]');
  await expect(dialogAfter).toBeVisible();
  await expect
    .poll(async () => (await dialogAfter.boundingBox())?.width)
    .toBe(initial.rect.w + 8);
});

// Multi-window arrangement persistence: a popped-out panel is recorded in the
// layout's `popouts`; a reload rehydrates it to a floating window (no gesture,
// no popup) while retaining the arrangement, and the restore notice's action —
// one click, itself the required user gesture — reopens the window with the
// saved panel set.
test("a popped-out panel's arrangement persists and the restore action reopens it", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Popout Restore World", account);

  const popupEvent = page.context().waitForEvent("page");
  const popoutPut = layoutPut(page, (l) => {
    const popouts = (
      l as { expanded?: { popouts?: { panels?: string[] }[] } }
    )?.expanded?.popouts;
    return (
      Array.isArray(popouts) &&
      popouts.some((w) => w.panels?.includes("chat:panel"))
    );
  });
  await chatTabMenu(page, "panel-menu-popOut");
  const popup = await popupEvent;
  await popup.waitForLoadState();
  expect(popup.url()).toContain("popout.html");
  await popoutPut;

  // Reload the main page WITHOUT closing the popup first: a window close would
  // emit the pop-in op and erase the arrangement record this test asserts on.
  // The reloaded app owns no handle to the pre-reload popup; it is closed
  // explicitly once the new session is up.
  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute(
    "data-render-ready",
    "true",
    {
      timeout: 30_000,
    },
  );
  await popup.close();

  // Rehydrated to floating, with the restore notice's action present.
  await expect(page.locator('[role="dialog"]')).toBeVisible();
  const restore = page.locator(".sc-notify-action", {
    hasText: "Reopen windows",
  });
  await expect(restore).toBeVisible();

  const restoredEvent = page.context().waitForEvent("page");
  await restore.click();
  const restored = await restoredEvent;
  await restored.waitForLoadState();
  expect(restored.url()).toContain("popout.html");
  await expect(restored.getByRole("button", { name: "Send" })).toBeVisible();
  await restored.close();
});
