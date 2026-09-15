// @vitest-environment node
import { describe, it, expect } from "vitest";
import { playThrowSound } from "./audioSeam";

describe("playThrowSound (pre-integration no-op)", () => {
  it("never throws for a sound id or null", () => {
    expect(() => playThrowSound("snd-1")).not.toThrow();
    expect(() => playThrowSound(null)).not.toThrow();
  });
});
