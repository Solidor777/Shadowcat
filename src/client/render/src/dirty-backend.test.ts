// @vitest-environment node
import { describe, it, expect, vi } from "vitest";
import { MockBackend } from "./backend.mock";
import { wrapDirtyTracking } from "./dirty-backend";

describe("wrapDirtyTracking", () => {
  it("calls onDirty and forwards for a drawing method", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.resize(800, 600);
    expect(onDirty).toHaveBeenCalledOnce();
    expect(real.size).toEqual({ width: 800, height: 600 });
  });

  it("does not call onDirty for tickTokenAnimations (called unconditionally every tick)", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.tickTokenAnimations(16);
    expect(onDirty).not.toHaveBeenCalled();
  });

  it("does not call onDirty for ensureLayers/addLayerFilter/startTicker/destroy", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.ensureLayers(["background"]);
    wrapped.addLayerFilter("background", {});
    wrapped.startTicker(() => {});
    wrapped.destroy();
    expect(onDirty).not.toHaveBeenCalled();
    expect(real.layers).toEqual(["background"]);
    expect(real.destroyed).toBe(true);
  });

  it("forwards setVisibilityBlend and calls onDirty when the real backend defines it", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    const input = { mode: "all" as const, visible: [], explored: [], perceived: [] };
    wrapped.setVisibilityBlend?.(input, input, 0.5);
    expect(onDirty).toHaveBeenCalledOnce();
    expect(real.visibility).toEqual(input);
  });

  it("does not call onDirty for setFrameCap/setRenderScale (settings, not draws)", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.setFrameCap(30);
    wrapped.setRenderScale(0.75);
    expect(onDirty).not.toHaveBeenCalled();
    expect(real.frameCap).toBe(30);
    expect(real.renderScale).toBe(0.75);
  });

  it("forwards render() without calling onDirty (render CONSUMES the flag, never sets it)", () => {
    const real = new MockBackend();
    const onDirty = vi.fn();
    const wrapped = wrapDirtyTracking(real, onDirty);
    wrapped.render();
    expect(onDirty).not.toHaveBeenCalled();
    expect(real.renderCount).toBe(1);
  });
});
