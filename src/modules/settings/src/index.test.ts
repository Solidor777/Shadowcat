import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { settings } from "./index";

describe("settings module", () => {
  it("contributes a panel with panel metadata after game-settings' order (5)", () => {
    expect(settings.manifest.id).toBe("settings");
    expect(settings.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    settings.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].order).toBe(6);
    expect(list[0].panel).toEqual({
      icon: "🔧",
      labelKey: "settings.tab",
    });
  });
});
