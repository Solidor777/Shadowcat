import { STAGE_OVERLAY_CONTRACT, type Module } from "@shadowcat/core";
import DiceOverlay from "./DiceOverlay.svelte";

/** 3D dice overlay module: contributes the `<DiceOverlay>` component
 * into `STAGE_OVERLAY_CONTRACT`, rendered by `Stage.svelte`'s `<Surface>`. */
export const dice3d: Module = {
  manifest: {
    id: "dice3d",
    version: "0.1.0",
    dependencies: {},
    requires: [],
    provides: [{ contract: STAGE_OVERLAY_CONTRACT, cardinality: "multi" }],
  },
  register(ctx) {
    ctx.contributions.contribute(
      { id: "dice3d:overlay", contract: STAGE_OVERLAY_CONTRACT, component: DiceOverlay },
    );
  },
};

export { remapFaces } from "./remapFaces";
export { seedFromRollId, mulberry32 } from "./rng";
export { shapeFor, realFaceCountOf } from "./shapes";
export type { DieShapeId, ResolvedShape } from "./shapes";
export { shapeGeometry } from "./geometry";
export type { ShapeFace, ShapeGeometry } from "./geometry";
export { readDice3DSettings, writeDice3DSettings } from "./settings";
export type { Dice3DDeviceSettings } from "./settings";
export { dice3dEnabled, reducedMotionPreferred, antialiasPreferred } from "./performanceSeam";
export { playThrowSound } from "./audioSeam";
export {
  MAX_CONCURRENT_ROLLS,
  MAX_DICE_PER_ROLL,
  createTriggerState,
  seedFromSnapshot,
  scanForPlays,
  toQueuedRoll,
  createRollQueue,
  enqueueRoll,
  dequeueRoll,
} from "./trigger";
export type { RollPlay, TriggerState, QueuedRoll, RollQueue, DequeueResult } from "./trigger";
export { DiceEngine } from "./DiceEngine";
export type { DieThrowSpec, SettledDie, DiceEngineOpts } from "./DiceEngine";
