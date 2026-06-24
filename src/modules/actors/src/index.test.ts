import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { actors } from "./index";

describe("actors module", () => {
  it("contributes a sidebar panel", () => {
    expect(actors.manifest.id).toBe("actors");
    expect(actors.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    actors.register({ contributions } as never);
    expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBe(1);
  });
});
