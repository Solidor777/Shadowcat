import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { assetBrowser } from "./index";

describe("asset-browser module", () => {
  it("contributes a GM-only panel with panel metadata", () => {
    expect(assetBrowser.manifest.id).toBe("asset-browser");
    expect(assetBrowser.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    assetBrowser.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].id).toBe("asset-browser:panel");
    expect(list[0].panel).toEqual({
      icon: "🖼️",
      labelKey: "assetBrowser.tab",
      gmOnly: true,
    });
  });
});
