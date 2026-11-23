import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, TOUR_CONTRACT } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import Settings from "./Settings.svelte";
import { settings } from "./index";
import { settingsTour } from "./tour";

describe("settings tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    settings.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["settings:tour"]);
    expect(tours[0].tour).toBe(settingsTour);
  });

  it("is replay-only, because the panel is closed until the user opens it", () => {
    expect(settingsTour.trigger).toBe("manual");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [settingsTour.labelKey, ...settingsTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets only anchors the settings panel mounts", () => {
    render(Settings, { context: setAppContextForTest({ role: "player" }) });
    for (const step of settingsTour.steps) expect(findTourAnchor(document, step.target), step.target).not.toBeNull();
  });
});
