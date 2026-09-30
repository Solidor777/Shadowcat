import { test, expect } from "@playwright/test";
import { latchAttributeValues, latchTextSeen } from "./transient-latch";

// Controls for the latches themselves: each must record a state that came and went before any
// poll could observe it, and must NOT report one that never happened.
test("the values latch records a state entered and left inside one task", async ({ page }) => {
  await page.setContent('<div id="host" data-state="idle"></div>');
  const host = page.locator("#host");
  const latch = await latchAttributeValues(host, "data-state");
  await host.evaluate((el) => {
    el.setAttribute("data-state", "settled");
    el.setAttribute("data-state", "idle");
  });
  expect(await host.getAttribute("data-state")).toBe("idle");
  expect(await latch.seen()).toEqual(["idle", "settled"]);
});

test("transitions grows on a repeated write of a value already seen, where a polled read cannot tell", async ({ page }) => {
  await page.setContent('<div id="host" data-state="idle"></div>');
  const host = page.locator("#host");
  const latch = await latchAttributeValues(host, "data-state");
  await host.evaluate((el) => {
    el.setAttribute("data-state", "tumbling");
    el.setAttribute("data-state", "idle");
    el.setAttribute("data-state", "tumbling");
    el.setAttribute("data-state", "idle");
  });
  const afterFirst = (await latch.transitions()).length;
  expect(await host.getAttribute("data-state")).toBe("idle");
  expect(await latch.transitions()).toEqual(["idle", "tumbling", "idle", "tumbling", "idle"]);
  await host.evaluate((el) => {
    el.setAttribute("data-state", "tumbling");
    el.setAttribute("data-state", "idle");
  });
  // The distinct-value view is unchanged by the repeat, the ordered history is not.
  expect(await latch.seen()).toEqual(["idle", "tumbling"]);
  expect((await latch.transitions()).length).toBe(afterFirst + 2);
  // With no write at all the history does not grow.
  const quiet = (await latch.transitions()).length;
  await page.waitForTimeout(100);
  expect((await latch.transitions()).length).toBe(quiet);
});

test("the values latch does not report a state that was never entered", async ({ page }) => {
  await page.setContent('<div id="host" data-state="idle"></div>');
  const host = page.locator("#host");
  const latch = await latchAttributeValues(host, "data-state");
  await host.evaluate((el) => el.setAttribute("data-state", "tumbling"));
  const states = await latch.seen();
  expect(states).toContain("tumbling");
  expect(states).not.toContain("settled");
});

test("the text latch records a notice added and removed before any poll", async ({ page }) => {
  await page.setContent("<body><p>ready</p></body>");
  const latch = await latchTextSeen(page, "Your turn");
  await page.evaluate(() => {
    const toast = document.createElement("div");
    toast.textContent = "Your turn";
    document.body.appendChild(toast);
    return new Promise<void>((resolve) => setTimeout(() => { toast.remove(); resolve(); }, 20));
  });
  expect(await page.getByText("Your turn").count()).toBe(0);
  expect(await latch.seen()).toBe(true);
});

test("the text latch records a notice added and removed inside one batch of mutations", async ({ page }) => {
  await page.setContent("<body><p>ready</p></body>");
  const latch = await latchTextSeen(page, "Your turn");
  await page.evaluate(() => {
    const toast = document.createElement("div");
    toast.textContent = "Your turn";
    document.body.appendChild(toast);
    toast.remove();
  });
  // A latch that only re-read the body text per callback sees an empty body here.
  expect(await page.evaluate(() => document.body.textContent?.includes("Your turn"))).toBe(false);
  expect(await latch.seen()).toBe(true);
});

test("the text latch stays false when the text never appears", async ({ page }) => {
  await page.setContent("<body><p>ready</p></body>");
  const latch = await latchTextSeen(page, "Your turn");
  await page.evaluate(() => {
    const other = document.createElement("div");
    other.textContent = "someone else";
    document.body.appendChild(other);
  });
  expect(await latch.seen()).toBe(false);
});
