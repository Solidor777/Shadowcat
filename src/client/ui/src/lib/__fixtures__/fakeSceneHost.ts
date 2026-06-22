import type { SceneToolHost, Point } from "@shadowcat/render";

/** A SceneToolHost with no-op defaults for tests; override the methods you assert on.
 * Centralizes the host shape so a new SceneToolHost method only updates this one place. */
export function fakeSceneHost(over: Partial<SceneToolHost> = {}): SceneToolHost {
  return {
    setActiveTool: () => {},
    snap: (p: Point) => p,
    setDraggingToken: () => {},
    previewOverlay: () => {},
    clearOverlay: () => {},
    gridDistance: () => 0,
    drawMeasure: () => {},
    clearMeasure: () => {},
    ...over,
  };
}
