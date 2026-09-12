import { describe, it, expect, vi } from "vitest";
import { PerformanceController } from "./performance.svelte";
import { PRESETS } from "@shadowcat/core";

describe("PerformanceController", () => {
  it("defaults to auto/balanced with no signals", () => {
    const c = new PerformanceController();
    expect(c.preset).toBe("auto");
    expect(c.current).toEqual(PRESETS.balanced);
  });

  it("load(undefined, signals) resolves auto against the given signals", () => {
    const c = new PerformanceController();
    c.load(undefined, { hardwareConcurrency: 2 });
    expect(c.preset).toBe("auto");
    expect(c.current).toMatchObject({ fpsCap: 30 });
  });

  it("load(parsed, signals) restores a named preset", () => {
    const c = new PerformanceController();
    c.load({ preset: "quality", overrides: {} }, {});
    expect(c.preset).toBe("quality");
    expect(c.current).toEqual(PRESETS.quality);
  });

  it("setPreset switches preset and clears overrides", () => {
    const c = new PerformanceController();
    c.set({ fpsCap: 30 });
    expect(c.preset).toBe("custom");
    c.setPreset("mobile");
    expect(c.preset).toBe("mobile");
    expect(c.current).toEqual(PRESETS.mobile);
  });

  it("set moves preset to custom, carrying the full current object forward", () => {
    const c = new PerformanceController();
    c.setPreset("quality");
    c.set({ fpsCap: 30 });
    expect(c.preset).toBe("custom");
    expect(c.current).toEqual({ ...PRESETS.quality, fpsCap: 30 });
  });

  it("set fires onChange with the serialized state", () => {
    const c = new PerformanceController();
    const onChange = vi.fn();
    c.onChange = onChange;
    c.set({ idleSkip: false });
    expect(onChange).toHaveBeenCalledWith({ preset: "custom", overrides: { ...PRESETS.balanced, idleSkip: false } });
  });

  it("setShowStats does not fire onChange (not part of PersistedPerformance)", () => {
    const c = new PerformanceController();
    const onChange = vi.fn();
    c.onChange = onChange;
    c.setShowStats(true);
    expect(c.showStats).toBe(true);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("recordStats updates stats without firing onChange or subscribers", () => {
    const c = new PerformanceController();
    const onChange = vi.fn();
    const listener = vi.fn();
    c.onChange = onChange;
    c.subscribe(listener);
    c.recordStats({ fps: 60, frameMs: 4 });
    expect(c.stats).toEqual({ fps: 60, frameMs: 4 });
    expect(onChange).not.toHaveBeenCalled();
    expect(listener).not.toHaveBeenCalled();
  });

  it("subscribe notifies on set/setPreset/load", () => {
    const c = new PerformanceController();
    const listener = vi.fn();
    const unsubscribe = c.subscribe(listener);
    c.setPreset("mobile");
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    c.setPreset("quality");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("serialize round-trips through load", () => {
    const c = new PerformanceController();
    c.set({ fpsCap: 30 });
    const snap = c.serialize();
    const c2 = new PerformanceController();
    c2.load(snap, {});
    expect(c2.current).toEqual(c.current);
  });
});
