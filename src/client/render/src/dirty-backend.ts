import type { DisplayBackend } from "./backend";

/** Wraps `backend` so every mutating draw call also invokes `onDirty()` before forwarding to the
 * real backend — the ONE seam `RenderEngine`'s idle-skip dirty flag hooks. Every reconciler/view
 * (`SceneReconciler`, `TokenView`, `DrawingView`, `TemplateView`, `WallView`, `RegionView`,
 * `LightView`) and both `Compositor`/`Lighting` already route every push through the injected
 * `DisplayBackend`, so intercepting at this one boundary needs no change to any of them.
 *
 * Excluded from dirty-tracking: `ensureLayers`/`addLayerFilter` (one-time/opt-in setup, not a
 * per-frame redraw trigger), `startTicker`/`destroy` (lifecycle, not drawing),
 * `tickTokenAnimations` (called UNCONDITIONALLY every tick by `TokenView.tick` regardless of
 * whether an animated sprite exists — wrapping it would mark every tick dirty and defeat
 * idle-skip entirely; `TokenView.hasAnimatedVisual` covers the real animated-sprite-redraw need
 * instead), and `setFrameCap`/`setRenderScale` (budget settings, not draws) and `render` itself
 * (the call that CONSUMES the dirty flag, never sets it).
 * @param backend The real backend to wrap.
 * @param onDirty Called synchronously before forwarding any dirty-tracked method's call.
 * @returns A `DisplayBackend` behaviorally identical to `backend`, reporting every draw.
 * @example
 * ```ts
 * import { MockBackend } from "@shadowcat/render";
 * import { wrapDirtyTracking } from "@shadowcat/render";
 *
 * let dirty = false;
 * const backend = wrapDirtyTracking(new MockBackend(), () => { dirty = true; });
 * backend.resize(800, 600); // dirty === true
 * ```
 */
export function wrapDirtyTracking(backend: DisplayBackend, onDirty: () => void): DisplayBackend {
  return {
    ensureLayers: (orderedIds) => backend.ensureLayers(orderedIds),
    setBackground: (spec) => { onDirty(); backend.setBackground(spec); },
    setClearColor: (color) => { onDirty(); backend.setClearColor(color); },
    drawGrid: (lines, color) => { onDirty(); backend.drawGrid(lines, color); },
    setVisibility: (input) => { onDirty(); backend.setVisibility(input); },
    setVisibilityBlend: backend.setVisibilityBlend
      ? (from, to, factor) => { onDirty(); backend.setVisibilityBlend!(from, to, factor); }
      : undefined,
    setCameraTransform: (t) => { onDirty(); backend.setCameraTransform(t); },
    addLayerFilter: (layerId, filter) => backend.addLayerFilter(layerId, filter),
    setToken: (id, spec) => { onDirty(); backend.setToken(id, spec); },
    removeToken: (id) => { onDirty(); backend.removeToken(id); },
    tickTokenAnimations: (dtMs) => backend.tickTokenAnimations(dtMs),
    setShape: (id, spec) => { onDirty(); backend.setShape(id, spec); },
    removeShape: (id) => { onDirty(); backend.removeShape(id); },
    drawOverlay: (shapes) => { onDirty(); backend.drawOverlay(shapes); },
    clearOverlay: () => { onDirty(); backend.clearOverlay(); },
    drawMeasure: (from, to, label) => { onDirty(); backend.drawMeasure(from, to, label); },
    clearMeasure: () => { onDirty(); backend.clearMeasure(); },
    drawPings: (rings) => { onDirty(); backend.drawPings(rings); },
    drawEmotes: (glyphs) => { onDirty(); backend.drawEmotes(glyphs); },
    setLighting: (frame) => { onDirty(); backend.setLighting(frame); },
    startTicker: (cb) => backend.startTicker(cb),
    resize: (width, height) => { onDirty(); backend.resize(width, height); },
    setFrameCap: (fps) => backend.setFrameCap(fps),
    setRenderScale: (scale) => backend.setRenderScale(scale),
    render: () => backend.render(),
    destroy: () => backend.destroy(),
  };
}
