import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { domController, mountAnchor } from "./__fixtures__/dom";
import { tour } from "./__fixtures__/harness";

const viewportFlag = vi.hoisted(() => ({ compact: true }));

// The breakpoint lives in ui-kit's `sizeClass`, read once at import from `matchMedia`, which jsdom
// lacks; the stand-in lets one test file render the narrow-screen branch.
vi.mock("@shadowcat/ui-kit", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@shadowcat/ui-kit")>()),
  sizeClass: () => (viewportFlag.compact ? "compact" : "expanded"),
}));

import TourOverlay from "./TourOverlay.svelte";

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

/** Renders the overlay over a started one-step tour.
 * @returns The render result and the controller.
 * @example
 * ```ts
 * const { view } = open();
 * ```
 */
function open() {
  mountAnchor("example.a");
  const { controller } = domController([tour()]);
  controller.start(tour());
  const view = render(TourOverlay, { props: { controller }, context: setAppContextForTest() });
  return { view, controller };
}

describe("TourOverlay on a narrow screen", () => {
  it("renders the card as a bottom sheet without computed coordinates", () => {
    viewportFlag.compact = true;
    const { view } = open();
    const card = view.getByTestId("tour-card");
    expect(card.classList.contains("sheet")).toBe(true);
    expect(card.style.left).toBe("");
    expect(card.style.top).toBe("");
  });

  it("dismisses on a downward touch swipe past the threshold", async () => {
    viewportFlag.compact = true;
    const { view, controller } = open();
    const card = view.getByTestId("tour-card");
    await fireEvent.pointerDown(card, { pointerType: "touch", clientY: 100 });
    await fireEvent.pointerUp(card, { pointerType: "touch", clientY: 200 });
    expect(controller.active).toBeNull();
  });

  it("ignores a short drag and a mouse drag", async () => {
    viewportFlag.compact = true;
    const { view, controller } = open();
    const card = view.getByTestId("tour-card");
    await fireEvent.pointerDown(card, { pointerType: "touch", clientY: 100 });
    await fireEvent.pointerUp(card, { pointerType: "touch", clientY: 120 });
    await fireEvent.pointerDown(card, { pointerType: "mouse", clientY: 100 });
    await fireEvent.pointerUp(card, { pointerType: "mouse", clientY: 300 });
    expect(controller.active).not.toBeNull();
  });
});

describe("TourOverlay on a wide screen", () => {
  it("positions the card next to the target instead of as a sheet", () => {
    viewportFlag.compact = false;
    const { view } = open();
    const card = view.getByTestId("tour-card");
    expect(card.classList.contains("sheet")).toBe(false);
    expect(card.style.left).not.toBe("");
    expect(card.style.top).not.toBe("");
  });
});
