import type { SceneToolHost, Point } from "@shadowcat/render";

/**
 * A SceneToolHost with no-op defaults for tests; override the methods you assert on.
 * Centralizes the host shape so a new SceneToolHost method only updates this one place.
 *
 * Fidelity gap: the defaults do NOT emulate the real `RenderEngine` host's grid math.
 * `snap` is identity (returns `p` unchanged) rather than snapping to the active grid's
 * nearest cell center, and `gridDistance` always returns `0` rather than a real
 * whole-cell distance. A test asserting on snap/measure OUTPUT must override those
 * methods explicitly; the defaults only satisfy the interface shape.
 * @param over - Per-method overrides; every field not given falls back to a no-op
 * (or, for `snap`, identity; for `gridDistance`, `0`).
 * @returns A `SceneToolHost` suitable for `ctx.scene`/render-engine test doubles.
 * @example fakeSceneHost({ snap: (p) => ({ x: Math.round(p.x), y: Math.round(p.y) }) });
 */
export function fakeSceneHost(over: Partial<SceneToolHost> = {}): SceneToolHost {
  return {
    setActiveTool: () => {},
    snap: (p: Point) => p,
    setSnapEnabled: () => {},
    setDraggingToken: () => {},
    previewOverlay: () => {},
    clearOverlay: () => {},
    gridDistance: () => 0,
    drawMeasure: () => {},
    clearMeasure: () => {},
    addPing: () => {},
    animateAlongPath: () => {},
    animateSamples: () => {},
    ...over,
  };
}
