import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT } from "@shadowcat/core";
import { gameSettings } from "./index";

describe("game-settings module", () => {
  it("contributes a GM-only panel with panel metadata", () => {
    expect(gameSettings.manifest.id).toBe("game-settings");
    expect(gameSettings.manifest.requires).toContain(PANEL_CONTRACT);
    const contributions = new ContributionRegistry();
    gameSettings.register({ contributions } as never);
    const list = contributions.contributionsFor(PANEL_CONTRACT);
    expect(list.length).toBe(1);
    expect(list[0].panel).toEqual({
      icon: "⚙️",
      labelKey: "gameSettings.tab",
      gmOnly: true,
    });
  });
});
