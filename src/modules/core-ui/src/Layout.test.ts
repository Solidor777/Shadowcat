import { test, expect, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Layout from "./Layout.svelte";
import layoutSource from "./Layout.svelte?raw";
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
// the row, the grid and every sibling past the viewport. `.statusbar` shares this cap even
// though its own row is a fixed `2rem` track, not `1fr`: a grid track sized by a fixed
// length still grows to fit an uncapped item's minimum content size (the same mechanism
// `.main`/`.toolrail` guard against), which is exactly what a tall `DockChips` contribution
// (`min-height: 44px`, the mobile touch-target floor) hosted in the statusbar row would
// otherwise do. Enumerated so a region added to either row without the cap fails here
// rather than in a real layout.
test("every region carrying the growth cap enforces it, 1fr row and fixed-row alike", () => {
  const { container } = render(Layout, { context: setAppContextForTest() });
  for (const selector of [".main", ".toolrail", ".statusbar"]) {
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

// jsdom performs no real layout (see the toolrail-column test's own note), so the fixed
// 2rem statusbar track's actual pixel height can't be read back here even after mounting a
// tall `DockChips`-shaped child — the growth-cap CSS declaration above is what pins the
// invariant in this environment; a real-viewport confirmation belongs to the e2e suite. This
// test instead confirms the cap survives a genuinely oversized child in the DOM: the
// `.statusbar` cell's OWN computed `min-height`/`overflow` (the properties that stop a grid
// track from growing) are unaffected by what is mounted inside it.
test("the statusbar's growth cap holds with an oversized (touch-target-floor) child mounted", async () => {
  const { ContributionRegistry } = await import("@shadowcat/core");
  const contributions = new ContributionRegistry();
  contributions.contribute({
    id: "test:statusbar-tall-chip",
    contract: "shadowcat.surface:statusbar",
    order: 0,
    component: OverlayProbe,
  });
  const { container } = render(Layout, {
    context: setAppContextForTest({ contributions }),
  });
  const el = container.querySelector(".statusbar");
  expect(el).toBeTruthy();
  const cs = getComputedStyle(el!);
  expect(cs.minHeight).toBe("0px");
  const overflowY = cs.overflowY === "visible" ? cs.overflow : cs.overflowY;
  expect(["hidden", "auto", "scroll", "clip"]).toContain(overflowY);
});

// jsdom performs no real layout, so the toolrail column's actual pixel width can't be read via
// getComputedStyle — it echoes the declared calc()/var() expression back unresolved (confirmed
// against a runtime probe render). This test instead reads the declared formula from source and
// evaluates it against the `--input-height-coarse`/`--space-1` token values (duplicated here as
// literals, mirroring their declared pixel values in the shell's shared `:root` primitives — a
// cross-package raw import of that stylesheet resolves to an empty string under this package's
// vitest/Vite pipeline, so the numbers can't be sourced live without that fragility instead).
test("the toolrail column derives from the touch-target floor, wide enough after its border for a full touch target", () => {
  const touchTargetPx = 44; // --input-height-coarse
  const space1Px = 4; // --space-1 (0.25rem at the shell's 16px root font-size)

  const columnMatch = layoutSource.match(
    /grid-template-columns:\s*calc\(var\(--input-height-coarse\)\s*\+\s*var\(--space-1\)\s*\*\s*(\d+)\s*\+\s*(\d+)px\)\s+1fr;/,
  );
  expect(columnMatch, "expected the toolrail column to derive from var(--input-height-coarse)").toBeTruthy();
  const paddingMultiplier = Number(columnMatch![1]);
  const borderPx = Number(columnMatch![2]);
  const columnWidthPx = touchTargetPx + space1Px * paddingMultiplier + borderPx;

  const cellBorderMatch = layoutSource.match(/\.toolrail\s*\{[^}]*border-right:\s*(\d+)px/);
  expect(cellBorderMatch, "expected `.toolrail`'s own border-right width").toBeTruthy();
  const cellBorderPx = Number(cellBorderMatch![1]);

  const contentBoxPx = columnWidthPx - cellBorderPx;
  expect(contentBoxPx).toBeGreaterThanOrEqual(touchTargetPx);
});
