// scene-tools active-tool state + the SceneTool implementations. Reaches the engine
// only through the public AppContext seams (the scene bridge for tool activation/snap,
// dispatchIntent for document writes); it never imports core-ui (contract-only
// boundary). The tool factories close over the context.
import type { SceneTool, Point } from "@shadowcat/render";
import type { DocumentStore, AssetResolver, WireOperation } from "@shadowcat/core";
import type { SceneInteraction } from "../../lib/sceneInteraction";

export type ToolId = "select" | "place";

/** The AppContext slice the tools need. */
export interface ToolContext {
  scene: SceneInteraction;
  dispatchIntent: (ops: WireOperation[]) => void;
  store: DocumentStore;
  assets: AssetResolver;
  world: string;
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

// TODO: place tool — click → create a token doc from the selected asset.
function makePlaceTool(_ctx: ToolContext, _controller: ToolController): SceneTool {
  return noopTool();
}

// TODO: select/move tool — pick + drag tokens → coalesced position intents.
function makeSelectMoveTool(_ctx: ToolContext, _controller: ToolController): SceneTool {
  return noopTool();
}
