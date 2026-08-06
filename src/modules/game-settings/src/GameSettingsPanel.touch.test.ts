import { describe, it, expect } from "vitest";
import gameSettingsPanelSource from "./GameSettingsPanel.svelte?raw";

describe("GameSettingsPanel touch sizing", () => {
  it("every input carries the shared coarse-pointer touch-sizing rule", () => {
    // jsdom doesn't evaluate @media (pointer: coarse), so assert the rule's
    // presence directly in the component's source styles instead (mirrors
    // "select/input controls get a 44px coarse-pointer min-height").
    const ruleMatch = gameSettingsPanelSource.match(/\binput\s*\{([^}]*@media[^}]*\{[^}]*\}[^}]*)\}/);
    expect(ruleMatch).toBeTruthy();
    expect(ruleMatch?.[1]).toMatch(/@media \(pointer: coarse\)\s*\{\s*min-height:\s*var\(--input-height-coarse\);\s*\}/);
  });
});
