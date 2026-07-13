import { describe, it, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { gameSettings } from "./index";

describe("game-settings module", () => {
  it("contributes a GM-only sidebar panel with tab metadata", () => {
    expect(gameSettings.manifest.id).toBe("game-settings");
    expect(gameSettings.manifest.requires).toContain("shadowcat.surface:sidebar");
    const contributions = new ContributionRegistry();
    gameSettings.register({ contributions } as never);
    const list = contributions.contributionsFor("shadowcat.surface:sidebar");
    expect(list.length).toBe(1);
    expect(list[0].tab).toEqual({ icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true });
  });
});
