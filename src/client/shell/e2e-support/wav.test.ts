// @vitest-environment node
import { describe, expect, it } from "vitest";
import { buildSilentWav, wavSecondsForBytes } from "./wav";

describe("buildSilentWav", () => {
  it("writes a canonical PCM16 header whose sizes describe the file", () => {
    const wav = buildSilentWav({ seconds: 0.5, sampleRate: 8000, channels: 1 });
    const data = 8000 * 0.5 * 2;
    expect(wav.length).toBe(44 + data);
    expect(wav.toString("ascii", 0, 4)).toBe("RIFF");
    expect(wav.readUInt32LE(4)).toBe(36 + data);
    expect(wav.toString("ascii", 8, 12)).toBe("WAVE");
    expect(wav.toString("ascii", 12, 16)).toBe("fmt ");
    expect(wav.readUInt32LE(16)).toBe(16);
    expect(wav.readUInt16LE(20)).toBe(1);
    expect(wav.readUInt16LE(22)).toBe(1);
    expect(wav.readUInt32LE(24)).toBe(8000);
    expect(wav.readUInt32LE(28)).toBe(8000 * 2);
    expect(wav.readUInt16LE(32)).toBe(2);
    expect(wav.readUInt16LE(34)).toBe(16);
    expect(wav.toString("ascii", 36, 40)).toBe("data");
    expect(wav.readUInt32LE(40)).toBe(data);
  });

  it("scales the byte rate and block align with the channel count", () => {
    const wav = buildSilentWav({ seconds: 1, sampleRate: 48000, channels: 2 });
    expect(wav.readUInt16LE(22)).toBe(2);
    expect(wav.readUInt32LE(28)).toBe(48000 * 4);
    expect(wav.readUInt16LE(32)).toBe(4);
    expect(wav.length).toBe(44 + 48000 * 4);
  });

  it("is silence: every sample byte is zero", () => {
    const wav = buildSilentWav({ seconds: 0.1, sampleRate: 8000, channels: 1 });
    expect(wav.subarray(44).every((b) => b === 0)).toBe(true);
  });
});

describe("wavSecondsForBytes", () => {
  it("returns the whole seconds whose file fits the target", () => {
    const seconds = wavSecondsForBytes(24 * 1024 * 1024, 48000, 2);
    const wav = buildSilentWav({ seconds, sampleRate: 48000, channels: 2 });
    expect(Number.isInteger(seconds)).toBe(true);
    expect(wav.length).toBeLessThanOrEqual(24 * 1024 * 1024);
    // One more second would not fit.
    expect(wav.length + 48000 * 4).toBeGreaterThan(24 * 1024 * 1024);
  });
});
