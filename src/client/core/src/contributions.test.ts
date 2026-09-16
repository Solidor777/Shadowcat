// @vitest-environment node
import { describe, it, expect, vi } from "vitest";
import { ContributionRegistry, type Contribution, PANEL_CONTRACT, type PanelMeta, SCENE_TOOL_CONTRACT, type SceneToolMeta } from "./contributions";

const c = (over: Partial<Contribution>): Contribution => ({
  id: "x",
  contract: "s:sidebar",
  component: {},
  ...over,
});

describe("ContributionRegistry", () => {
  it("returns contributions for a contract sorted by order then insertion", () => {
    const r = new ContributionRegistry();
    r.contribute(c({ id: "b", order: 2 }));
    r.contribute(c({ id: "a", order: 1 }));
    r.contribute(c({ id: "c" })); // order undefined → 0
    expect(r.contributionsFor("s:sidebar").map((x) => x.id)).toEqual(["c", "a", "b"]);
    expect(r.contributionsFor("s:other")).toEqual([]);
  });

  it("dispose removes a single contribution and notifies subscribers", () => {
    const r = new ContributionRegistry();
    const listener = vi.fn();
    r.subscribe(listener);
    const dispose = r.contribute(c({ id: "a" }));
    expect(listener).toHaveBeenCalledTimes(1);
    expect(r.contributionsFor("s:sidebar")).toHaveLength(1);
    dispose();
    expect(listener).toHaveBeenCalledTimes(2);
    expect(r.contributionsFor("s:sidebar")).toHaveLength(0);
  });

  it("removeModule drops every contribution tagged with that module", () => {
    const r = new ContributionRegistry();
    r.contribute(c({ id: "a" }), { module: "m1" });
    r.contribute(c({ id: "b" }), { module: "m1" });
    r.contribute(c({ id: "k" }), { module: "m2" });
    r.removeModule("m1");
    expect(r.contributionsFor("s:sidebar").map((x) => x.id)).toEqual(["k"]);
  });

  it("round-trips panel metadata through the shadowcat.panel contract, order-sorted", () => {
    const r = new ContributionRegistry();
    const panelB: PanelMeta = {
      icon: "b-icon",
      labelKey: "panel.b",
      defaultPlacement: { kind: "docked", zone: "right", order: 1 },
    };
    const panelA: PanelMeta = {
      icon: "a-icon",
      labelKey: "panel.a",
      gmOnly: true,
      defaultPlacement: { kind: "minimized" },
    };
    r.contribute(c({ id: "b", contract: PANEL_CONTRACT, order: 2, panel: panelB }));
    r.contribute(c({ id: "a", contract: PANEL_CONTRACT, order: 1, panel: panelA }));
    const result = r.contributionsFor(PANEL_CONTRACT);
    expect(result.map((x) => x.id)).toEqual(["a", "b"]);
    expect(result[0].panel).toEqual(panelA);
    expect(result[1].panel).toEqual(panelB);
  });

  it("subscribe returns an unsubscribe that stops notifications", () => {
    const r = new ContributionRegistry();
    const listener = vi.fn();
    const off = r.subscribe(listener);
    off();
    r.contribute(c({ id: "a" }));
    expect(listener).not.toHaveBeenCalled();
  });

  it("round-trips scene-tool metadata through SCENE_TOOL_CONTRACT, order-sorted, multi-module", () => {
    const r = new ContributionRegistry();
    const toolB: SceneToolMeta = { id: "tb", icon: "b", labelKey: "tool.b", onSceneClick: () => {} };
    const toolA: SceneToolMeta = { id: "ta", icon: "a", labelKey: "tool.a", onSceneClick: () => {} };
    r.contribute(c({ id: "m2:b", contract: SCENE_TOOL_CONTRACT, order: 2, sceneTool: toolB }), { module: "m2" });
    r.contribute(c({ id: "m1:a", contract: SCENE_TOOL_CONTRACT, order: 1, sceneTool: toolA }), { module: "m1" });
    const result = r.contributionsFor(SCENE_TOOL_CONTRACT);
    expect(result.map((x) => x.id)).toEqual(["m1:a", "m2:b"]);
    expect(result[0].sceneTool).toEqual(toolA);
    expect(result[1].sceneTool).toEqual(toolB);
  });
});

describe("ContributionRegistry.entriesFor", () => {
  it("returns each contribution paired with its registering module id", () => {
    const reg = new ContributionRegistry();
    reg.contribute({ id: "a", contract: "c", component: 1, sheet: { priority: 5 } }, { module: "mod-a" });
    reg.contribute({ id: "b", contract: "c", component: 2 }, { module: "mod-b" });
    reg.contribute({ id: "z", contract: "other", component: 3 }, { module: "mod-z" });
    const entries = reg.entriesFor("c");
    expect(entries.map((e) => [e.contribution.id, e.module])).toEqual([
      ["a", "mod-a"],
      ["b", "mod-b"],
    ]);
    expect(entries[0].contribution.sheet?.priority).toBe(5);
  });
});
