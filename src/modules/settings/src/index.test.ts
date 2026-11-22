import { describe, it, expect } from "vitest";
import { ContributionRegistry, PANEL_CONTRACT, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
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

  it("contributes the backup manager as a server-admin-only settings section", () => {
    const contributions = new ContributionRegistry();
    settings.register({ contributions } as never);
    const backups = contributions
      .contributionsFor(SETTINGS_SECTION_CONTRACT)
      .find((c) => c.id === "settings:backups");
    expect(backups?.settingsSection).toEqual({
      labelKey: "settings.backups.title",
      serverAdminOnly: true,
    });
  });

  it("contributes the port-forward section as a server-admin-only settings section", () => {
    const contributions = new ContributionRegistry();
    settings.register({ contributions } as never);
    const section = contributions
      .contributionsFor(SETTINGS_SECTION_CONTRACT)
      .find((c) => c.id === "settings:port-forward");
    expect(section?.settingsSection).toEqual({
      labelKey: "settings.portForward.title",
      serverAdminOnly: true,
    });
  });

  it("provides shadowcat.settings-section for other modules to contribute into", () => {
    expect(settings.manifest.provides).toEqual([
      { contract: "shadowcat.settings-section", cardinality: "multi" },
    ]);
  });
});
