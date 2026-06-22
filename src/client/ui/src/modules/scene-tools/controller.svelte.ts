// scene-tools active-tool state + the SceneTool implementations. Reaches the engine
// only through the public AppContext seams (the scene bridge for tool activation/snap,
// dispatchIntent for document writes); it never imports core-ui (contract-only
// boundary). The tool factories close over the context.
import type { SceneTool, Point } from "@shadowcat/render";
import { buildTokenDoc, type ReadableDocuments, type AssetResolver, type WireOperation } from "@shadowcat/core";
import type { SceneInteraction } from "../../lib/sceneInteraction";

export type ToolId = "select" | "place";

/** The AppContext slice the tools need. `documents` is the optimistic view, so a
 * just-auto-created scene / just-placed token is visible to the tools immediately. */
export interface ToolContext {
  scene: SceneInteraction;
  dispatchIntent: (ops: WireOperation[]) => void;
  documents: ReadableDocuments;
  assets: AssetResolver;
  world: string;
}

/** The active scene (single scene in M8d §15) + its grid cell size (default 100). */
function activeScene(ctx: ToolContext): { id: string; size: number } | null {
  const scene = ctx.documents.query("scene")[0];
  if (!scene) return null;
  const size = (scene.system as { grid?: { size?: number } } | undefined)?.grid?.size ?? 100;
  return { id: scene.id, size };
}

/** Owns the active-tool + selected-asset UI state and routes activation to the engine
 * via the scene bridge. */
export class ToolController {
  active = $state<ToolId | null>(null);
  /** The token art the place tool stamps; chosen in the asset picker. */
  selectedAsset = $state<string | null>(null);
  readonly #tools: Record<ToolId, SceneTool>;

  constructor(private readonly ctx: ToolContext) {
    this.#tools = {
      select: makeSelectMoveTool(ctx, this),
      place: makePlaceTool(ctx, this),
    };
  }

  /** Toggle a tool: re-selecting the active one clears it (back to camera). */
  toggle(id: ToolId): void {
    this.active = this.active === id ? null : id;
    this.ctx.scene.setActiveTool(this.active ? this.#tools[this.active] : null);
  }
}

const noopTool = (): SceneTool => ({
  onPointerDown: (_p: Point): boolean => false,
  onPointerMove: (): void => {},
  onPointerUp: (): void => {},
});

/** Click stamps a token (the selected asset) at the snapped cell of the active scene.
 * No scene or no selected asset → unhandled (the camera pans instead). */
export function makePlaceTool(ctx: ToolContext, controller: ToolController): SceneTool {
  return {
    onPointerDown(p: Point): boolean {
      const scene = activeScene(ctx);
      const asset = controller.selectedAsset;
      if (!scene || !asset) return false;
      const c = ctx.scene.snap(p);
      ctx.dispatchIntent([
        {
          op: "create",
          doc: buildTokenDoc(ctx.world, scene.id, {
            x: c.x,
            y: c.y,
            w: scene.size,
            h: scene.size,
            rotation: 0,
            visual: { kind: "image", asset },
          }),
        },
      ]);
      return true;
    },
    onPointerMove(): void {},
    onPointerUp(): void {},
  };
}

// TODO: select/move tool — pick + drag tokens → coalesced position intents.
function makeSelectMoveTool(_ctx: ToolContext, _controller: ToolController): SceneTool {
  return noopTool();
}
