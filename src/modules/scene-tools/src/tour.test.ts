import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, TOUR_CONTRACT } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ToolRail from "./ToolRail.svelte";
import { sceneTools } from "./index";
import { sceneToolsTour } from "./tour";

describe("scene tools tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    sceneTools.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["scene-tools:tour"]);
    expect(tours[0].tour).toBe(sceneToolsTour);
  });

  it("is offered on first use to everyone", () => {
    expect(sceneToolsTour.trigger).toBe("first-use");
    expect(sceneToolsTour.audience).toBe("any");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [sceneToolsTour.labelKey, ...sceneToolsTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets only anchors a GM's rail mounts", () => {
    render(ToolRail, { context: setAppContextForTest({ role: "gm" }) });
    for (const step of sceneToolsTour.steps) expect(findTourAnchor(document, step.target), step.target).not.toBeNull();
  });

  it("leaves the GM-only place step unmounted for a player", () => {
    render(ToolRail, { context: setAppContextForTest({ role: "player" }) });
    const mounted = sceneToolsTour.steps.filter((s) => findTourAnchor(document, s.target) !== null).map((s) => s.target);
    expect(mounted).toEqual(["scene-tools.rail", "scene-tools.select", "scene-tools.measure", "scene-tools.ping"]);
  });
});
