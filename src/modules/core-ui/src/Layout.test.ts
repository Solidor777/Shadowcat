import { test, expect, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Layout from "./Layout.svelte";
import OverlayProbe from "./__fixtures__/OverlayProbe.svelte";

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

// DOM order must follow compact's visual order (main before toolrail) so
// keyboard/screen-reader traversal reaches main content before tool controls;
// grid-template-areas alone govern visual placement in both modes.
test("the main region precedes the toolrail region in DOM order", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  const main = container.querySelector(".main");
  const toolrail = container.querySelector(".toolrail");
  expect(main).toBeTruthy();
  expect(toolrail).toBeTruthy();
  expect(main?.compareDocumentPosition(toolrail!)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
});

// The overlay layer hosts app-level modal chrome; a contribution into it must
// render OUTSIDE the grid so it is never clipped by a region's overflow.
test("renders overlay-surface contributions outside the layout grid", async () => {
  const { ContributionRegistry } = await import("@shadowcat/core");
  const contributions = new ContributionRegistry();
  contributions.contribute({
    id: "test:overlay-probe",
    contract: "shadowcat.surface:overlay",
    order: 0,
    component: OverlayProbe,
  });
  const { container } = render(Layout, {
    context: setAppContextForTest({ contributions }),
  });
  const probe = container.querySelector("[data-testid='overlay-probe']");
  expect(probe).toBeTruthy();
  expect(probe?.closest(".layout")).toBeNull();
});

// The middle grid row is `1fr` of a `100vh` grid, and a row is at least as tall as its
// tallest item's minimum contribution — so EVERY item in that row must carry the growth
// cap (`min-height: 0` plus a non-visible `overflow-y`), or that one item's content grows
// the row, the grid and every sibling past the viewport. Enumerated so a region added to
// the row without the cap fails here rather than in a real layout.
test("every region sharing the 1fr row carries the growth cap", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  for (const selector of [".main", ".toolrail"]) {
    const el = container.querySelector(selector);
    expect(el, selector).toBeTruthy();
    const cs = getComputedStyle(el!);
    expect(cs.minHeight, `${selector} min-height`).toBe("0px");
    // jsdom's cascade keeps a declared `overflow` shorthand as-is rather than expanding it
    // into the longhands, so a region declaring the shorthand reads through that fallback.
    const overflowY = cs.overflowY === "visible" ? cs.overflow : cs.overflowY;
    expect(["hidden", "auto", "scroll", "clip"], `${selector} overflow-y`).toContain(overflowY);
  }
});
