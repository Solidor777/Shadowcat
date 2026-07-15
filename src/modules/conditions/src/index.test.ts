import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { conditions } from "./index";

describe("conditions module", () => {
  it("contributes a panel and requires the panel-manager contract", () => {
    expect(conditions.manifest.id).toBe("conditions");
    expect(conditions.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    conditions.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].panel).toEqual({
      icon: "✨",
      labelKey: "conditions.tab",
    });
  });
});
