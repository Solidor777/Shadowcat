import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { assets } from "./index";

describe("assets module", () => {
  it("contributes a panel with panel metadata", () => {
    expect(assets.manifest.id).toBe("assets");
    expect(assets.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    assets.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].panel).toEqual({
      icon: "🖼️",
      labelKey: "assets.tab",
    });
  });
});
