import { test, expect, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Layout from "./Layout.svelte";

afterEach(() => cleanup());

test("renders the four region cells inside the layout grid", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  const layout = container.querySelector(".layout");
  expect(layout).toBeTruthy();
  expect(container.querySelector(".topbar")).toBeTruthy();
  expect(container.querySelector(".toolrail")).toBeTruthy();
  expect(container.querySelector(".main")).toBeTruthy();
  expect(container.querySelector(".statusbar")).toBeTruthy();
});

// jsdom has no matchMedia, so `sizeClass()` resolves to "expanded"; the compact
// grid (bottom tool strip) is asserted by the e2e viewport test, not here.
test("defaults to the expanded grid (no compact class) under jsdom", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  expect(container.querySelector(".layout")?.classList.contains("compact")).toBe(false);
});
