// @vitest-environment node
import { describe, it, expect } from "vitest";
import {
  PRESETS,
  resolveAuto,
  parsePersisted,
  serializePersisted,
  effectiveSettings,
  PERFORMANCE_STORAGE_KEY,
  fpsCapToTickerValue,
  type DeviceSignals,
  type PersistedPerformance,
} from "./performance";

describe("resolveAuto", () => {
  it("resolves balanced with no signals", () => {
    expect(resolveAuto({})).toEqual(PRESETS.balanced);
  });
  it("resolves mobile when coarsePointer AND compact", () => {
    expect(resolveAuto({ coarsePointer: true, compact: true })).toMatchObject({ fpsCap: 30, renderScale: 0.75 });
  });
  it("does not resolve mobile from coarsePointer alone (compact absent)", () => {
    expect(resolveAuto({ coarsePointer: true })).toEqual(PRESETS.balanced);
  });
  it("resolves mobile when hardwareConcurrency <= 4", () => {
    expect(resolveAuto({ hardwareConcurrency: 4 })).toMatchObject({ fpsCap: 30 });
  });
  it("does not resolve mobile when hardwareConcurrency > 4", () => {
    expect(resolveAuto({ hardwareConcurrency: 8 })).toEqual(PRESETS.balanced);
  });
  it("resolves mobile when deviceMemoryGb <= 4", () => {
    expect(resolveAuto({ deviceMemoryGb: 4 })).toMatchObject({ fpsCap: 30 });
  });
  it("ORs reducedMotion onto the resolved preset", () => {
    expect(resolveAuto({ reducedMotion: true })).toEqual({ ...PRESETS.balanced, reducedMotion: true });
  });
});

describe("textureBudgetBytes", () => {
  it("is smaller on mobile than on balanced/quality (a floor on GPU memory, not a ceiling raise)", () => {
    expect(PRESETS.mobile.textureBudgetBytes).toBeLessThan(PRESETS.balanced.textureBudgetBytes);
    expect(PRESETS.mobile.textureBudgetBytes).toBeLessThan(PRESETS.quality.textureBudgetBytes);
  });

  it("is a positive finite byte count on every preset", () => {
    for (const preset of Object.values(PRESETS)) {
      expect(preset.textureBudgetBytes).toBeGreaterThan(0);
      expect(Number.isFinite(preset.textureBudgetBytes)).toBe(true);
    }
  });

  it("survives a parsePersisted/serializePersisted round trip as a custom override", () => {
    const p: PersistedPerformance = { preset: "custom", overrides: { textureBudgetBytes: 123_456_789 } };
    expect(parsePersisted(serializePersisted(p))).toEqual(p);
  });

  it("sanitizeOverrides drops a non-finite or non-positive textureBudgetBytes", () => {
    expect(parsePersisted(JSON.stringify({ preset: "custom", overrides: { textureBudgetBytes: -1 } })).overrides)
      .toEqual({});
    expect(parsePersisted(JSON.stringify({ preset: "custom", overrides: { textureBudgetBytes: Number.NaN } })).overrides)
      .toEqual({});
  });
});

describe("parsePersisted", () => {
  it("returns the auto default for null", () => {
    expect(parsePersisted(null)).toEqual({ preset: "auto", overrides: {} });
  });
  it("returns the auto default for unparsable JSON", () => {
    expect(parsePersisted("not json")).toEqual({ preset: "auto", overrides: {} });
  });
  it("returns the auto default for a non-object payload", () => {
    expect(parsePersisted("42")).toEqual({ preset: "auto", overrides: {} });
  });
  it("falls back preset to auto when unresolvable, keeps valid overrides", () => {
    expect(parsePersisted(JSON.stringify({ preset: "bogus", overrides: { fpsCap: 60 } })))
      .toEqual({ preset: "auto", overrides: { fpsCap: 60 } });
  });
  it("drops a single bad override key and keeps the rest", () => {
    const raw = JSON.stringify({
      preset: "custom",
      overrides: { fpsCap: 999, renderScale: 0.8, antialias: "yes" },
    });
    expect(parsePersisted(raw)).toEqual({ preset: "custom", overrides: { renderScale: 0.8 } });
  });
  it("clamps renderScale on parse", () => {
    const raw = JSON.stringify({ preset: "custom", overrides: { renderScale: 5 } });
    expect(parsePersisted(raw).overrides.renderScale).toBe(1);
  });
});

describe("effectiveSettings", () => {
  const signals: DeviceSignals = {};
  it("resolves auto through resolveAuto", () => {
    expect(effectiveSettings({ preset: "auto", overrides: {} }, signals)).toEqual(resolveAuto(signals));
  });
  it("resolves a named preset verbatim", () => {
    expect(effectiveSettings({ preset: "quality", overrides: {} }, signals)).toEqual(PRESETS.quality);
  });
  it("custom overrides win over the balanced fallback base", () => {
    const p: PersistedPerformance = { preset: "custom", overrides: { fpsCap: 120 } };
    expect(effectiveSettings(p, signals)).toEqual({ ...PRESETS.balanced, fpsCap: 120 });
  });
  it("ORs the live reducedMotion signal onto every preset", () => {
    expect(effectiveSettings({ preset: "quality", overrides: {} }, { reducedMotion: true }).reducedMotion).toBe(true);
  });
  it("clamps renderScale at read time regardless of source", () => {
    const p: PersistedPerformance = { preset: "custom", overrides: { renderScale: 0.1 } };
    expect(effectiveSettings(p, signals).renderScale).toBe(0.5);
  });
});

it("serializePersisted round-trips through parsePersisted", () => {
  const p: PersistedPerformance = { preset: "custom", overrides: { fpsCap: 30, idleSkip: false } };
  expect(parsePersisted(serializePersisted(p))).toEqual(p);
});

it("PERFORMANCE_STORAGE_KEY is the expected literal", () => {
  expect(PERFORMANCE_STORAGE_KEY).toBe("shadowcat.performance");
});

describe("fpsCapToTickerValue", () => {
  it("maps uncapped to 0 and passes numeric caps through", () => {
    expect(fpsCapToTickerValue("uncapped")).toBe(0);
    expect(fpsCapToTickerValue(30)).toBe(30);
    expect(fpsCapToTickerValue(60)).toBe(60);
    expect(fpsCapToTickerValue(120)).toBe(120);
  });
});
