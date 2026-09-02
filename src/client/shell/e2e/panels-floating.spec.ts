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

// Multi-panel arrangement restore: a window saved with TWO panels — the exact
// persisted shape a drag-into-popout gesture produces (a `popouts` record whose
// `panels` list grew via `popOutInto`) — rehydrates both panels to floating on
// reload, and the restore notice's action reopens ONE popup hosting both, with
// the resulting arrangement persisted again. The record is seeded as a
// leaf-level ui-state patch rather than produced by a drag, because a real
// cross-window HTML5 tab drag cannot be driven from Playwright (browser input
// dispatch is per-page; a drag session cannot span two windows) — the
// drag-into-popout gesture itself is covered by the engine unit tests, and the
// mechanism under test HERE (restore gesture → ops → tree → popup → persist)
// runs for real.
test("the restore action reopens a multi-panel arrangement into one window hosting every saved panel", async ({
  page,
  account,
}) => {
  await enterFreshWorld(page, "Popout Multi Restore World", account);
  const worldId = new URL(page.url()).hash.replace(/^#\/world\//, "");
  expect(worldId).not.toBe("");

  // Wait out the shell's ui-state persist cooldown (`schedulePersist`) before
  // seeding, so no boot persist lands after the seed and clobbers it; no
  // gestures follow the seed, so nothing schedules another `panelLayout`
  // write before the reload.
  await page.waitForTimeout(700);
  const seedLayout = {
    version: 1,
    expanded: {
      zones: {
        right: { groups: [], size: 320 },
        bottom: { groups: [], size: 240 },
        left: { groups: [], size: 320 },
      },
      floating: [],
      minimized: [],
      popouts: [
        { key: "w-e2e", panels: ["chat:panel", "asset-browser:panel"], rect: null },
      ],
    },
    compact: { activeView: null, order: [] },
  };
  const seedResponse = await page.request.put("/api/me/ui-state", {
    data: { worlds: { [worldId]: { panelLayout: seedLayout } } },
  });
  expect(seedResponse.ok()).toBe(true);

  await page.reload();
  await expect(page.locator(".stage-host")).toHaveAttribute(
    "data-render-ready",
    "true",
    {
      timeout: 30_000,
    },
  );

  // Both saved panels rehydrated to floating (one dialog each), and the
  // restore notice's action is offered.
  await expect(page.locator('[role="dialog"]')).toHaveCount(2);
  const restore = page.locator(".sc-notify-action", {
    hasText: "Reopen windows",
  });
  await expect(restore).toBeVisible();

  const restoredPut = layoutPut(page, (l) => {
    const popouts = (
      l as { expanded?: { popouts?: { panels?: string[] }[] } }
    )?.expanded?.popouts;
    return (
      Array.isArray(popouts) &&
      popouts.some(
        (w) =>
          w.panels?.includes("chat:panel") &&
          w.panels?.includes("asset-browser:panel"),
      )
    );
  });
  const restoredEvent = page.context().waitForEvent("page");
  await restore.click();
  const restored = await restoredEvent;
  await restored.waitForLoadState();
  expect(restored.url()).toContain("popout.html");

  // ONE popup hosting BOTH panels: a tab each in its tab strip. The restore
  // moves the second panel in last, so it starts active — activating the Chat
  // tab must surface chat's own composer inside the popup.
  await expect(restored.locator(".dv-tab", { hasText: "Chat" })).toBeVisible();
  await expect(
    restored.locator(".dv-tab", { hasText: "Assets" }),
  ).toBeVisible();
  await restored.locator(".dv-tab", { hasText: "Chat" }).click();
  await expect(restored.getByRole("button", { name: "Send" })).toBeVisible();

  // The restore's popOut + popOutInto ops round-tripped through the tree and
  // persisted one window record listing both panels.
  await restoredPut;
  await restored.close();
});
