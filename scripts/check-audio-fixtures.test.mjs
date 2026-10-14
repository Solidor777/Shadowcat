import { describe, expect, it } from "vitest";
import {
  levelFailures,
  parseIntegratedLufs,
  parseMaxVolume,
  byteEqualityEnforceable,
} from "./check-audio-fixtures.mjs";

describe("audio fixture level parsing", () => {
  it("reads volumedetect's max_volume", () => {
    expect(parseMaxVolume("[Parsed_volumedetect_0 @ 0x1] max_volume: -11.9 dB\n")).toBe(-11.9);
    expect(parseMaxVolume("nothing")).toBeNull();
  });

  it("reads the integrated loudness from the summary only", () => {
    const stderr =
      "[Parsed_ebur128_0 @ 0x1] t: 1 TARGET:-23 LUFS M: -50 S: -40 I: -99.0 LUFS LRA: 0.0 LU\n" +
      "[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -22.9 LUFS\n    Threshold: -33.0 LUFS\n";
    expect(parseIntegratedLufs(stderr)).toBe(-22.9);
    expect(parseIntegratedLufs("none")).toBeNull();
  });
});

describe("levelFailures", () => {
  it("passes a quiet in-band track", () => {
    expect(levelFailures(-11.9, -22.9)).toEqual([]);
  });

  it("fails at a peak over the bound, at each loudness edge, and on a missing reading", () => {
    expect(levelFailures(-10.9, -22.9)).toHaveLength(1);
    expect(levelFailures(-12, -19.9)).toHaveLength(1);
    expect(levelFailures(-12, -26.1)).toHaveLength(1);
    expect(levelFailures(-11, -20)).toEqual([]);
    expect(levelFailures(-11, -26)).toEqual([]);
    expect(levelFailures(null, null)).toHaveLength(2);
  });
});

describe("byteEqualityEnforceable", () => {
  const manifest = { ffmpeg_version: "8.1.1", numpy_version: "2.1.2" };

  it("holds only when both recorded versions match", () => {
    expect(byteEqualityEnforceable(manifest, { ffmpeg_version: "8.1.1", numpy_version: "2.1.2" })).toBe(true);
    expect(byteEqualityEnforceable(manifest, { ffmpeg_version: "6.1.1", numpy_version: "2.1.2" })).toBe(false);
    expect(byteEqualityEnforceable(manifest, { ffmpeg_version: "8.1.1", numpy_version: "1.26.4" })).toBe(false);
  });
});
