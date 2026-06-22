// The canvas interaction bridge (M8d §16). A stable handle owned by WorldSession and
// exposed on AppContext, so tool components reach the engine's tool API even though the
// RenderEngine is created lazily inside the Stage effect. Stage attaches the engine on
// mount; before/after attachment every call no-ops (snap is identity) so a tool
// component never crashes when no canvas is mounted. Render types are type-only imports
// (zero runtime dependency on @shadowcat/render here).
import type { SceneTool, SceneToolHost, Point } from "@shadowcat/render";

/** The host-facing seam plus late-attachment. */
export interface SceneInteraction extends SceneToolHost {
  /** Attach the live engine (a SceneToolHost); returns a detach that only clears the
   * host if it is still the current one (a stale detach after re-attach is a no-op). */
  attach(host: SceneToolHost): () => void;
}

export class SceneInteractionBridge implements SceneInteraction {
  #host: SceneToolHost | null = null;

  attach(host: SceneToolHost): () => void {
    this.#host = host;
    return () => {
      if (this.#host === host) this.#host = null;
    };
  }

  setActiveTool(tool: SceneTool | null): void {
    this.#host?.setActiveTool(tool);
  }

  snap(p: Point): Point {
    return this.#host ? this.#host.snap(p) : p;
  }

  setDraggingToken(id: string | null): void {
    this.#host?.setDraggingToken(id);
  }
}
