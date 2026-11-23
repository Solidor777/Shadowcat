import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import { ContributionRegistry, TOUR_CONTRACT, type TourMeta } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ReplaySection from "./ReplaySection.svelte";
import { domController, mountAnchor } from "./__fixtures__/dom";
import { tour } from "./__fixtures__/harness";

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
});

/** Renders the replay list over a registry holding `tours`.
 * @param tours The tours to contribute.
 * @param role The viewer's world role.
 * @param notify The notification spy.
 * @returns The render result and the controller.
 * @example
 * ```ts
 * const { view } = list([tour()], "gm", vi.fn());
 * ```
 */
function list(tours: TourMeta[], role: "gm" | "player", notify: (message: string) => void) {
  const contributions = new ContributionRegistry();
  tours.forEach((t) => contributions.contribute({ id: `tour:${t.id}`, contract: TOUR_CONTRACT, component: null, tour: t }));
  const { controller } = domController(tours);
  const view = render(ReplaySection, {
    props: { controller },
    context: setAppContextForTest({ contributions, role, notify }),
  });
  return { view, controller };
}

describe("ReplaySection", () => {
  it("lists every tour for the viewer, finished or not, by its label", () => {
    const tours = [tour({ id: "example.one" }), tour({ id: "example.two", audience: "player" }), tour({ id: "example.gm", audience: "gm" })];
    const { view } = list(tours, "gm", vi.fn());
    expect(view.getByTestId("tour-replay-example.one")).toBeTruthy();
    expect(view.getByTestId("tour-replay-example.gm")).toBeTruthy();
    expect(view.queryByTestId("tour-replay-example.two")).toBeNull();
  });

  it("lists a manual tour too", () => {
    const { view } = list([tour({ trigger: "manual" })], "gm", vi.fn());
    expect(view.getByTestId("tour-replay-example.intro")).toBeTruthy();
  });

  it("shows an empty note when no tour is for the viewer", () => {
    const { view } = list([tour({ audience: "gm" })], "player", vi.fn());
    expect(view.getByText("onboarding.replay.empty")).toBeTruthy();
  });

  it("starts the tour when Replay is pressed", async () => {
    mountAnchor("example.a");
    const { view, controller } = list([tour()], "gm", vi.fn());
    await fireEvent.click(view.getByTestId("tour-replay-example.intro"));
    expect(controller.active?.tour.id).toBe("example.intro");
  });

  it("tells the user when the tour has no step on screen", async () => {
    const notify = vi.fn();
    const { view, controller } = list([tour()], "gm", notify);
    await fireEvent.click(view.getByTestId("tour-replay-example.intro"));
    expect(controller.active).toBeNull();
    expect(notify).toHaveBeenCalledWith("onboarding.replay.unavailable", "info");
  });

  it("shows a tour contributed after mount", async () => {
    const contributions = new ContributionRegistry();
    const { controller } = domController([]);
    const view = render(ReplaySection, { props: { controller }, context: setAppContextForTest({ contributions }) });
    expect(view.queryByTestId("tour-replay-example.intro")).toBeNull();
    contributions.contribute({ id: "tour:late", contract: TOUR_CONTRACT, component: null, tour: tour() });
    expect(await view.findByTestId("tour-replay-example.intro")).toBeTruthy();
  });
});
