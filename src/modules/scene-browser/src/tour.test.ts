import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { ContributionRegistry, DocumentStore, TOUR_CONTRACT, TourMetaSchema, buildSceneDoc } from "@shadowcat/core";
import { findTourAnchor, t } from "@shadowcat/ui-kit";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import SceneBrowserPanel from "./SceneBrowserPanel.svelte";
import { sceneBrowser } from "./index";
import { sceneBrowserTour } from "./tour";

// The panel lists assets on mount; jsdom has no server to answer.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return { ...actual, listAssets: vi.fn().mockResolvedValue([]) };
});

/** A store holding two scenes. */
function twoScenes(): DocumentStore {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "u",
    ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", {}, "sA") },
      { op: "create", doc: buildSceneDoc("w1", {}, "sB") },
    ],
  });
  return store;
}

describe("scene-browser tour", () => {
  it("is contributed under the tour contract and passes registry validation", () => {
    const contributions = new ContributionRegistry();
    sceneBrowser.register({ contributions } as never);
    const tours = contributions.contributionsFor(TOUR_CONTRACT);
    expect(tours.map((c) => c.id)).toEqual(["scene-browser:tour"]);
    expect(tours[0].tour).toBe(sceneBrowserTour);
    expect(TourMetaSchema.safeParse(sceneBrowserTour).success).toBe(true);
  });

  it("is for GMs and replay-only, because the panel is GM-only and closed until opened", () => {
    expect(sceneBrowserTour.audience).toBe("gm");
    expect(sceneBrowserTour.trigger).toBe("manual");
  });

  it("names only strings that resolve in the English locale", () => {
    const keys = [sceneBrowserTour.labelKey, ...sceneBrowserTour.steps.flatMap((s) => [s.titleKey, s.bodyKey])];
    for (const key of keys) expect(t(key), key).not.toBe(key);
  });

  it("targets the anchors the panel mounts, leaving the actions step to a world with a scene", () => {
    render(SceneBrowserPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore() }) });
    const mounted = sceneBrowserTour.steps.filter((s) => findTourAnchor(document, s.target) !== null).map((s) => s.target);
    expect(mounted).toEqual(["scene-browser.panel", "scene-browser.create", "scene-browser.list"]);
  });

  it("anchors the actions of the first scene row only", () => {
    const documents = twoScenes();
    render(SceneBrowserPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents, store: documents }) });
    expect(document.querySelectorAll('[data-tour="scene-browser.actions"]')).toHaveLength(1);
    expect(findTourAnchor(document, "scene-browser.actions")?.closest("li")).toBe(document.querySelector("li"));
  });
});
