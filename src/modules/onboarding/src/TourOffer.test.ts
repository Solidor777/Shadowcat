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

  it("Not now dismisses and records the tour as done", async () => {
    const { view, controller, settings } = await offered();
    await fireEvent.click(view.getByTestId("tour-offer-later"));
    expect(controller.offer).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 1 }]);
  });

  it("Don't ask again records the switch", async () => {
    const { view, controller, settings } = await offered();
    await fireEvent.click(view.getByTestId("tour-offer-never"));
    expect(controller.offer).toBeNull();
    expect(settings.writes).toEqual([{ disabled: true }]);
  });
});
