import { describe, it, expect, beforeEach } from "vitest";
import { readDice3DSettings, writeDice3DSettings } from "./settings";

describe("Dice3D device settings", () => {
  beforeEach(() => localStorage.clear());

  it("defaults when nothing is stored", () => {
    expect(readDice3DSettings()).toEqual({ color: "", labelColor: "", material: "plastic" });
  });

  it("round-trips a written value", () => {
    writeDice3DSettings({ color: "#2d6ee8", labelColor: "#ffffff", material: "glass" });
    expect(readDice3DSettings()).toEqual({ color: "#2d6ee8", labelColor: "#ffffff", material: "glass" });
  });

  it("falls back to defaults on garbage JSON", () => {
    localStorage.setItem("shadowcat.dice3d", "{not json");
    expect(readDice3DSettings()).toEqual({ color: "", labelColor: "", material: "plastic" });
  });

  it("falls back on an invalid material value", () => {
    localStorage.setItem("shadowcat.dice3d", JSON.stringify({ color: "", labelColor: "", material: "wood" }));
    expect(readDice3DSettings().material).toBe("plastic");
  });
});
