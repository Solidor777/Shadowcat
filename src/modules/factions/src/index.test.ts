import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { factions } from "./index";

describe("factions module", () => {
  it("contributes a panel and requires the panel-manager contract", () => {
    expect(factions.manifest.id).toBe("factions");
    expect(factions.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    factions.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].panel).toEqual({
      icon: "🚩",
      labelKey: "factions.tab",
    });
  });
});
