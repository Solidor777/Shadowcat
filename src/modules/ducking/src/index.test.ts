import { describe, it, expect, afterEach } from "vitest";
import { ContributionRegistry, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
import { ducking } from "./index";

describe("ducking module", () => {
  afterEach(() => {
    ducking.unregister?.();
    localStorage.clear();
  });

  it("requires SETTINGS_SECTION_CONTRACT and provides nothing", () => {
    expect(ducking.manifest.requires).toEqual(["shadowcat.settings-section"]);
    expect(ducking.manifest.provides).toEqual([]);
  });

  it("contributes its settings section with the expected metadata", () => {
    const contributions = new ContributionRegistry();
    ducking.register({ contributions } as never);
    const list = contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
    expect(list).toHaveLength(1);
    expect(list[0].id).toBe("ducking:settings");
    expect(list[0].settingsSection).toEqual({ labelKey: "ducking.sectionTitle" });
  });
});
