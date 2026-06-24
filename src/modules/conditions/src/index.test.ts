import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { conditions } from "./index";

describe("conditions module", () => {
  it("contributes a sidebar panel and requires the sidebar surface", () => {
    expect(conditions.manifest.id).toBe("conditions");
    expect(conditions.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    conditions.register({ contributions } as never);
    expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBe(1);
  });
});
