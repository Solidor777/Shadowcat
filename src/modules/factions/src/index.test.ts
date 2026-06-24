import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { factions } from "./index";

describe("factions module", () => {
  it("contributes a sidebar panel and requires the sidebar surface", () => {
    expect(factions.manifest.id).toBe("factions");
    expect(factions.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    factions.register({ contributions } as never);
    expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBe(1);
  });
});
