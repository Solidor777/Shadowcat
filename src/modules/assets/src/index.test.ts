import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { assets } from "./index";

describe("assets module", () => {
  it("contributes a sidebar panel with tab metadata", () => {
    expect(assets.manifest.id).toBe("assets");
    expect(assets.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    assets.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:sidebar");
    expect(list.length).toBe(1);
    expect(list[0].tab).toEqual({ icon: "🖼️", labelKey: "assets.tab" });
  });
});
