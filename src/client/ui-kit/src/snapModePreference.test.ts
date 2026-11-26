// @vitest-environment node
// Exercises plain state and pure functions: no component render and no DOM API use, so
// the package-default jsdom environment would be constructed per file and never touched.
import { describe, it, expect } from "vitest";
import { SnapModePreference } from "./snapModePreference.svelte";

describe("SnapModePreference", () => {
  it("defaults to center, preserving cell-center snapping unless a GM opts into vertex mode", () => {
    expect(new SnapModePreference().mode).toBe("center");
  });

  it("set updates the mode in place", () => {
    const s = new SnapModePreference();
    s.set("vertex");
    expect(s.mode).toBe("vertex");
    s.set("center");
    expect(s.mode).toBe("center");
  });
});
