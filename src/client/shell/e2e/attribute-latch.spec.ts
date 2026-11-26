import { test, expect } from "@playwright/test";
import { latchAttributeMax } from "./attribute-latch";

// A control for the latch itself: it must record a rise that reverted before any poll could
// observe it, which is exactly the case a polling assertion cannot distinguish from "never rose".
test("the latch records a value that rose and fell inside one task", async ({ page }) => {
  await page.setContent('<div id="host" data-count="1"></div>');
  const host = page.locator("#host");
  const latch = await latchAttributeMax(host, "data-count");
  await host.evaluate((el) => {
    el.setAttribute("data-count", "2");
    el.setAttribute("data-count", "1");
  });
  expect(await host.getAttribute("data-count")).toBe("1");
  expect(await latch.max()).toBe(2);
});

test("the latch stays at the resting value when the attribute never rises", async ({ page }) => {
  await page.setContent('<div id="host" data-count="1"></div>');
  const host = page.locator("#host");
  const latch = await latchAttributeMax(host, "data-count");
  await host.evaluate((el) => {
    el.setAttribute("data-count", "0");
    el.setAttribute("data-count", "1");
  });
  expect(await latch.max()).toBe(1);
});

test("an absent or non-numeric value counts as zero", async ({ page }) => {
  await page.setContent("<div id=\"host\"></div>");
  const host = page.locator("#host");
  const latch = await latchAttributeMax(host, "data-count");
  await host.evaluate((el) => el.setAttribute("data-count", "not-a-number"));
  expect(await latch.max()).toBe(0);
});
