import { describe, it, expect } from "vitest";
import gameSettingsPanelSource from "./GameSettingsPanel.svelte?raw";

describe("GameSettingsPanel touch sizing", () => {
  it("every input and the reset-to-system button carry the shared coarse-pointer touch-sizing rule", () => {
    // jsdom doesn't evaluate @media (pointer: coarse), so assert the rule's
    // presence directly in the component's source styles instead (mirrors
    // "select/input controls get a 44px coarse-pointer min-height"). The selector list
    // extends the single `input` rule to also cover `.reset-to-system`, so the pattern
    // allows (but does not require) additional comma-separated selectors before the brace.
    const ruleMatch = gameSettingsPanelSource.match(/\binput\s*(?:,[^{]*)?\{([^}]*@media[^}]*\{[^}]*\}[^}]*)\}/);
    expect(ruleMatch).toBeTruthy();
    expect(ruleMatch?.[1]).toMatch(/@media \(pointer: coarse\)\s*\{\s*min-height:\s*var\(--input-height-coarse\);\s*\}/);
    // Assert the selector list explicitly includes the new reset button, not just that
    // SOME selector list precedes the rule.
    const selectorMatch = gameSettingsPanelSource.match(/\binput\s*,[^{]*\.reset-to-system[^{]*\{/);
    expect(selectorMatch).toBeTruthy();
  });
});
