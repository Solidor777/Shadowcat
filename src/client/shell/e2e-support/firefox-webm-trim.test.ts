// @vitest-environment node
import { describe, expect, it } from "vitest";
import { discardPaddingNanos, firefoxWebmDecodeDelta } from "./firefox-webm-trim";

const BASE = 50 * 960;

describe("firefoxWebmDecodeDelta", () => {
  // Residues and deltas measured in Firefox's decodeAudioData over the server's WebM files: the
  // audible count F of the last packet is 312 + residue up to 648 and residue - 648 + 960 - 649
  // beyond (F = 1 at 649, 311 at 959).
  it.each([
    [0, 0],
    [3, 0],
    [4, -1],
    [5, -1],
    [9, 0],
    [33, -1],
    [51, -1],
    [648, 0],
    [649, 0],
    [959, -1],
  ])("predicts the measured delta for residue %i", (residue, delta) => {
    expect(firefoxWebmDecodeDelta(BASE + residue)).toBe(delta);
  });

  it("is 0 or -1 everywhere and follows F mod 6 outside the float-noise case", () => {
    for (let residue = 0; residue < 960; residue++) {
      const valid = BASE + residue;
      const frames = Math.ceil((312 + valid) / 960);
      const audible = 960 - (frames * 960 - 312 - valid);
      const delta = firefoxWebmDecodeDelta(valid);
      expect([0, -1]).toContain(delta);
      if (frames * 960 - 312 - valid === 0) expect(delta).toBe(0);
      else if (audible % 6 === 4 || audible % 6 === 5) expect(delta).toBe(-1);
      else if (audible % 6 !== 3) expect(delta).toBe(0);
    }
  });
});

describe("discardPaddingNanos", () => {
  it("rounds the padding up to a whole nanosecond and is 0 on an exact fit", () => {
    expect(discardPaddingNanos(BASE + 648)).toBe(0);
    expect(discardPaddingNanos(BASE)).toBe(13_500_000);
    expect(discardPaddingNanos(BASE + 959)).toBe(13_520_834);
  });
});
