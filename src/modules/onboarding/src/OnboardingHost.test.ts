import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { ContributionRegistry, TOUR_CONTRACT, type TourMeta } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import OnboardingHost from "./OnboardingHost.svelte";
import { TourController } from "./controller.svelte";
import { TourProgress } from "./progress";
import { memorySettings, tour, type MemorySettings } from "./__fixtures__/harness";
import { mountAnchor } from "./__fixtures__/dom";

/** Stubs `fetch` so `GET /api/config` answers with the given switch.
 * @param enabled The `onboarding_enabled` value to report.
 * @example
 * ```ts
 * serverReports(true);
 * ```
 */
function serverReports(enabled: boolean): void {
  vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ onboarding_enabled: enabled })));
}

beforeEach(() => serverReports(true));

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  document.body.innerHTML = "";
});

/** Mounts the host the way a session does: a fresh controller over `settings`, tours in a real
 * registry, anchors in the document.
 * @param tours The tours to contribute.
 * @param settings The account settings; share one across calls to model a second session.
 * @returns The render result.
 * @example
 * ```ts
 * const view = session([tour()], memorySettings());
 * ```
 */
function session(tours: TourMeta[], settings: MemorySettings) {
  const contributions = new ContributionRegistry();
  tours.forEach((t) => contributions.contribute({ id: `tour:${t.id}`, contract: TOUR_CONTRACT, component: null, tour: t }));
  const controller = new TourController({
    progress: new TourProgress(settings, () => {}),
    serverEnabled: async () => (await fetch("/api/config").then((r) => r.json())).onboarding_enabled !== false,
  });
  const view = render(OnboardingHost, { props: { controller }, context: setAppContextForTest({ contributions, role: "gm" }) });
  return { view, controller, contributions };
}

describe("OnboardingHost", () => {
  it("offers a tour to a fresh user, runs it, and does not offer it to the next session", async () => {
    mountAnchor("example.a");
    const settings = memorySettings();
    const first = session([tour()], settings);
    await waitFor(() => expect(first.view.getByTestId("tour-offer")).toBeTruthy());

    await fireEvent.click(first.view.getByTestId("tour-offer-start"));
    expect(first.view.queryByTestId("tour-offer")).toBeNull();
    await fireEvent.click(first.view.getByTestId("tour-next"));
    expect(first.view.queryByTestId("tour-overlay")).toBeNull();
    expect(settings.slice).toEqual({ "g:example.intro": 1 });
    first.view.unmount();

    const second = session([tour()], settings);
    await second.controller.evaluate();
    expect(second.view.queryByTestId("tour-offer")).toBeNull();
  });

  it("Disable Tutorials persists across sessions", async () => {
    mountAnchor("example.a");
    const settings = memorySettings();
    const first = session([tour()], settings);
    await waitFor(() => expect(first.view.getByTestId("tour-offer")).toBeTruthy());
    await fireEvent.click(first.view.getByTestId("tour-offer-never"));
    expect(first.view.queryByTestId("tour-offer")).toBeNull();
    first.view.unmount();

    const second = session([tour()], settings);
    await second.controller.evaluate();
    expect(second.view.queryByTestId("tour-offer")).toBeNull();
    expect(settings.slice).toEqual({ disabled: true, "g:example.intro": 1 });
  });

  it("Not Now hides the toast and the tour is not offered again in a later session", async () => {
    mountAnchor("example.a");
    const settings = memorySettings();
    const first = session([tour()], settings);
    await waitFor(() => expect(first.view.getByTestId("tour-offer")).toBeTruthy());
    await fireEvent.click(first.view.getByTestId("tour-offer-later"));
    expect(first.view.queryByTestId("tour-offer")).toBeNull();
    expect(settings.writes).toEqual([{ "g:example.intro": 1 }]);
    first.view.unmount();

    const second = session([tour()], settings);
    await second.controller.evaluate();
    expect(second.view.queryByTestId("tour-offer")).toBeNull();
  });

  it("each answer ends in the instructions for reaching tours later, which Got it closes", async () => {
    for (const answer of ["tour-offer-later", "tour-offer-never"]) {
      mountAnchor("example.a");
      const { view } = session([tour()], memorySettings());
      await waitFor(() => expect(view.getByTestId("tour-offer")).toBeTruthy());
      await fireEvent.click(view.getByTestId(answer));
      expect(view.getByTestId("tour-hint").textContent).toContain("onboarding.hint.message");
      await fireEvent.click(view.getByTestId("tour-hint-dismiss"));
      expect(view.queryByTestId("tour-hint")).toBeNull();
      view.unmount();
    }
  });

  it("after Take Tour the instructions wait until the tour has ended", async () => {
    mountAnchor("example.a");
    const { view } = session([tour()], memorySettings());
    await waitFor(() => expect(view.getByTestId("tour-offer")).toBeTruthy());
    await fireEvent.click(view.getByTestId("tour-offer-start"));
    expect(view.queryByTestId("tour-hint")).toBeNull();
    await fireEvent.click(view.getByTestId("tour-next"));
    expect(view.getByTestId("tour-hint")).toBeTruthy();
  });

  it("offers nothing when the server switch is off", async () => {
    serverReports(false);
    mountAnchor("example.a");
    const { view, controller } = session([tour()], memorySettings());
    await controller.evaluate();
    expect(view.queryByTestId("tour-offer")).toBeNull();
  });

  it("offers a tour contributed after mount", async () => {
    mountAnchor("example.a");
    const { view, contributions, controller } = session([], memorySettings());
    await controller.evaluate();
    expect(view.queryByTestId("tour-offer")).toBeNull();
    contributions.contribute({ id: "tour:late", contract: TOUR_CONTRACT, component: null, tour: tour() });
    await waitFor(() => expect(view.getByTestId("tour-offer")).toBeTruthy());
  });

  it("offers nothing when no step target is on screen", async () => {
    const { view, controller } = session([tour()], memorySettings());
    await controller.evaluate();
    expect(view.queryByTestId("tour-offer")).toBeNull();
  });
});
