import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, TOUR_CONTRACT } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Stage from "./Stage.svelte";
import { stage } from "./index";
import { stageTour } from "./tour";

/** Props that keep the stage from creating a real WebGL backend; the init never settles. */
const pendingBackend = { createBackend: vi.fn(() => new Promise<never>(() => {})) };

describe("stage tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    stage.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["stage:tour"]);
    expect(tours[0].tour).toBe(stageTour);
  });

  it("is offered on first use to everyone", () => {
    expect(stageTour.trigger).toBe("first-use");
    expect(stageTour.audience).toBe("any");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [stageTour.labelKey, ...stageTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets only anchors a GM's stage mounts", () => {
    render(Stage, { props: pendingBackend, context: setAppContextForTest({ role: "gm", subscribeScene: () => ({ unsubscribe() {} }) }) });
    for (const step of stageTour.steps) expect(findTourAnchor(document, step.target), step.target).not.toBeNull();
  });

  it("leaves the vision picker step unmounted for a player", () => {
    render(Stage, { props: pendingBackend, context: setAppContextForTest({ role: "player", subscribeScene: () => ({ unsubscribe() {} }) }) });
    expect(findTourAnchor(document, "stage.canvas")).not.toBeNull();
    expect(findTourAnchor(document, "stage.gm-view")).toBeNull();
  });
});
