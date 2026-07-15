import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { actors } from "./index";

describe("actors module", () => {
  it("contributes a panel", () => {
    expect(actors.manifest.id).toBe("actors");
    expect(actors.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    actors.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].panel).toEqual({
      icon: "👥",
      labelKey: "actors.tab",
    });
  });
});
