import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { settings } from "./index";

describe("settings module", () => {
  it("contributes a sidebar panel with tab metadata after game-settings' order (5)", () => {
    expect(settings.manifest.id).toBe("settings");
    expect(settings.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    settings.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:sidebar");
    expect(list.length).toBe(1);
    expect(list[0].order).toBe(6);
    expect(list[0].tab).toEqual({ icon: "🔧", labelKey: "settings.tab" });
  });
});
