export type { Point, LineSeg, Polygon, CameraTransform } from "./types";
export { LayerRegistry, CORE_LAYERS, type CoreLayerId } from "./layers";
export { Camera } from "./camera";
export { Grid, type GridKind, type GridSpec } from "./grid";
export type { DisplayBackend } from "./backend";
export { MockBackend } from "./backend.mock";
export { SceneReconciler } from "./reconciler";
export { RenderEngine, type RenderEngineOpts } from "./engine";
export { PixiBackend, createPixiBackend } from "./pixi-backend";
