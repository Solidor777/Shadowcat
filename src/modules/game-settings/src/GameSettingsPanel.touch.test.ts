// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import gameSettingsPanelSource from "./GameSettingsPanel.svelte?raw";

describe("GameSettingsPanel touch sizing", () => {
  it("every input and every plain button carry the shared coarse-pointer touch-sizing rule", () => {
    // jsdom doesn't evaluate @media (pointer: coarse), so assert the rule's
    // presence directly in the component's source styles instead (mirrors
    // "select/input controls get a 44px coarse-pointer min-height"). The selector list
    // extends the single `input` rule to also cover the bare `button` element — every
    // button in this file (`.reset-to-system` included, since it IS a `<button>`) rather
    // than one class name at a time — so the pattern allows (but does not require)
    // additional comma-separated selectors before the brace.
    const ruleMatch = gameSettingsPanelSource.match(/\binput\s*(?:,[^{]*)?\{([^}]*@media[^}]*\{[^}]*\}[^}]*)\}/);
    expect(ruleMatch).toBeTruthy();
    expect(ruleMatch?.[1]).toMatch(/@media \(pointer: coarse\)\s*\{\s*min-height:\s*var\(--input-height-coarse\);\s*\}/);
    // Assert the selector list explicitly includes the bare `button` element, not just that
    // SOME selector list precedes the rule — this is what covers every plain
    // `<button type="button">` in the file (ambienceClear, visionModeAdd, gradationAdd,
    // gradationRemoveFor, visionModeRemoveFor, reset-explored-fog, reset-to-system, and the
    // ambience-hit picker buttons), not only the one class name a narrower rule would name.
    const selectorMatch = gameSettingsPanelSource.match(/\binput\s*,\s*button\s*\{/);
    expect(selectorMatch).toBeTruthy();
  });
});
