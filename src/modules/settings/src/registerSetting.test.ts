import { describe, it, expect } from "vitest";
import { ContributionRegistry, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
import { registerSetting, type SettingField } from "./registerSetting";
import SettingRow from "./SettingRow.svelte";

describe("registerSetting", () => {
  const field: SettingField = { kind: "toggle", get: () => false, set: () => {} };

  it("contributes exactly one settings-section entry carrying the id, label key and field", () => {
    const contributions = new ContributionRegistry();
    registerSetting({ contributions }, { id: "example:flag", labelKey: "example.flag", field });
    const entries = contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
    expect(entries).toHaveLength(1);
    expect(entries[0].id).toBe("example:flag");
    expect(entries[0].component).toBe(SettingRow);
    expect(entries[0].settingsSection).toEqual({ labelKey: "example.flag" });
    expect(entries[0].props).toEqual({ field, labelKey: "example.flag" });
  });

  it("returns a dispose function that removes the contribution", () => {
    const contributions = new ContributionRegistry();
    const dispose = registerSetting({ contributions }, { id: "example:flag", labelKey: "example.flag", field });
    dispose();
    expect(contributions.contributionsFor(SETTINGS_SECTION_CONTRACT)).toHaveLength(0);
  });
});
