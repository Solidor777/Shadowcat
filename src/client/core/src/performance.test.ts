// @vitest-environment node
import { describe, it, expect } from "vitest";
import {
  PRESETS,
  resolveAuto,
  parsePersisted,
  serializePersisted,
  effectiveSettings,
  PERFORMANCE_STORAGE_KEY,
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
