import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import TourOffer from "./TourOffer.svelte";
import { makeController, tour } from "./__fixtures__/harness";

afterEach(cleanup);

/** Renders the toast over a controller that has made its offer.
 * @returns The render result, controller and settings.
 * @example
 * ```ts
 * const { view } = await offered();
 * ```
 */
async function offered() {
  const handle = makeController({ tours: [tour()], mounted: ["example.a"] });
  await handle.controller.evaluate();
  const view = render(TourOffer, { props: { controller: handle.controller }, context: setAppContextForTest() });
  return { ...handle, view };
}

describe("TourOffer", () => {
  it("shows the three choices and takes no focus", async () => {
    const { view } = await offered();
    expect(view.getByTestId("tour-offer-start")).toBeTruthy();
    expect(view.getByTestId("tour-offer-later")).toBeTruthy();
    expect(view.getByTestId("tour-offer-never")).toBeTruthy();
    expect(document.activeElement).toBe(document.body);
  });

  it("Start runs the tour", async () => {
    const { view, controller } = await offered();
    await fireEvent.click(view.getByTestId("tour-offer-start"));
    expect(controller.active?.tour.id).toBe("example.intro");
  });

  it("Not Now dismisses and records the tour as done", async () => {
    const { view, controller, settings } = await offered();
    await fireEvent.click(view.getByTestId("tour-offer-later"));
    expect(controller.offer).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 1 }]);
  });

  it("Disable Tutorials records the switch and the tour as done", async () => {
    const { view, controller, settings } = await offered();
    await fireEvent.click(view.getByTestId("tour-offer-never"));
    expect(controller.offer).toBeNull();
    expect(settings.slice).toEqual({ disabled: true, "g:example.intro": 1 });
  });

  it("labels the three buttons Take Tour, Not Now and Disable Tutorials", async () => {
    const { view } = await offered();
    expect(view.getByTestId("tour-offer-start").textContent?.trim()).toBe("onboarding.offer.start");
    expect(view.getByTestId("tour-offer-later").textContent?.trim()).toBe("onboarding.offer.later");
    expect(view.getByTestId("tour-offer-never").textContent?.trim()).toBe("onboarding.offer.never");
  });
});
