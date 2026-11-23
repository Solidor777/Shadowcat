import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import TourOverlay from "./TourOverlay.svelte";
import { domController, mountAnchor } from "./__fixtures__/dom";
import { threeStepTour, tour } from "./__fixtures__/harness";

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

/** Renders the overlay over a started tour.
 * @param over The tour to run.
 * @returns The render result, the controller and its settings.
 * @example
 * ```ts
 * const { controller } = open(tour());
 * ```
 */
function open(over = threeStepTour()) {
  const handle = domController([over]);
  expect(handle.controller.start(over)).toBe(true);
  const view = render(TourOverlay, {
    props: { controller: handle.controller },
    context: setAppContextForTest(),
  });
  return { ...handle, view };
}

describe("TourOverlay content", () => {
  it("shows the step's title, body and position as an accessible dialog", () => {
    mountAnchor("example.a");
    mountAnchor("example.b");
    const { view } = open();
    const card = view.getByTestId("tour-card");
    expect(card.getAttribute("role")).toBe("dialog");
    expect(card.getAttribute("aria-modal")).toBe("true");
    expect(view.getByText("example.a.title")).toBeTruthy();
    expect(view.getByText("example.a.body")).toBeTruthy();
    expect(view.getByTestId("tour-count").textContent).toBe("onboarding.step");
  });

  it("walks the shown steps with Next and Back and offers Done on the last", async () => {
    mountAnchor("example.a");
    mountAnchor("example.c");
    const { view, controller } = open();
    expect(view.getByTestId("tour-next").textContent?.trim()).toBe("onboarding.next");
    expect((view.getByTestId("tour-back") as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(view.getByTestId("tour-next"));
    expect(controller.active?.stepIndex).toBe(2);
    expect(view.getByText("example.c.title")).toBeTruthy();
    expect(view.getByTestId("tour-next").textContent?.trim()).toBe("onboarding.done");
    await fireEvent.click(view.getByTestId("tour-back"));
    expect(controller.active?.stepIndex).toBe(0);
  });

  it("Done ends the tour and records it", async () => {
    mountAnchor("example.a");
    const { view, controller, settings } = open(tour({ version: 2 }));
    await fireEvent.click(view.getByTestId("tour-next"));
    expect(controller.active).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 2 }]);
  });

  it("Skip ends the tour and records it", async () => {
    mountAnchor("example.a");
    mountAnchor("example.b");
    const { view, controller, settings } = open();
    await fireEvent.click(view.getByTestId("tour-skip"));
    expect(controller.active).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 1 }]);
  });
});

describe("TourOverlay spotlight", () => {
  it("surrounds the anchor's first element child with padding", () => {
    const target = mountAnchor("example.a");
    target.getBoundingClientRect = () => ({ left: 10, top: 20, width: 30, height: 40 }) as DOMRect;
    const { view } = open(tour());
    const hole = view.getByTestId("tour-spotlight");
    expect(hole.style.left).toBe("6px");
    expect(hole.style.top).toBe("16px");
    expect(hole.style.width).toBe("38px");
    expect(hole.style.height).toBe("48px");
  });

  it("drops the spotlight when the anchor unmounts and the window resizes", async () => {
    mountAnchor("example.a");
    const { view } = open(tour());
    expect(view.queryByTestId("tour-spotlight")).not.toBeNull();
    document.querySelector("[data-tour]")?.remove();
    await fireEvent(window, new Event("resize"));
    expect(view.queryByTestId("tour-spotlight")).toBeNull();
    expect(view.getByTestId("tour-card")).toBeTruthy();
  });
});

describe("TourOverlay keyboard and focus", () => {
  it("focuses the primary button when it opens", () => {
    mountAnchor("example.a");
    const { view } = open(tour());
    expect(document.activeElement).toBe(view.getByTestId("tour-next"));
  });

  it("Escape ends the tour like Skip", async () => {
    mountAnchor("example.a");
    const { controller, settings } = open(tour());
    await fireEvent.keyDown(window, { key: "Escape" });
    expect(controller.active).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 1 }]);
  });

  it("traps Tab inside the card", async () => {
    mountAnchor("example.a");
    mountAnchor("example.b");
    const { view } = open();
    const card = view.getByTestId("tour-card");
    const next = view.getByTestId("tour-next");
    expect(document.activeElement).toBe(next);
    await fireEvent.keyDown(card, { key: "Tab" });
    expect(document.activeElement).toBe(view.getByTestId("tour-skip"));
    const skip = view.getByTestId("tour-skip");
    skip.focus();
    await fireEvent.keyDown(card, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(next);
  });

  it("returns focus to the element that had it before the tour opened", () => {
    mountAnchor("example.a");
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const { view } = open(tour());
    expect(document.activeElement).not.toBe(opener);
    view.unmount();
    expect(document.activeElement).toBe(opener);
  });
});
