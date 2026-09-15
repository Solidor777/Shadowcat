import { describe, it, expect, vi, afterEach } from "vitest";
import { dice3dEnabled, reducedMotionPreferred, antialiasPreferred } from "./performanceSeam";

describe("performanceSeam (pre-integration device-local getters)", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("dice3dEnabled defaults on", () => {
    expect(dice3dEnabled()).toBe(true);
  });

  it("antialiasPreferred defaults on", () => {
    expect(antialiasPreferred()).toBe(true);
  });

  it("reducedMotionPreferred reads prefers-reduced-motion", () => {
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("reduce") }) as MediaQueryList);
    expect(reducedMotionPreferred()).toBe(true);
  });

  it("reducedMotionPreferred is false when matchMedia is unavailable", () => {
    vi.stubGlobal("matchMedia", undefined);
    expect(reducedMotionPreferred()).toBe(false);
  });
});
