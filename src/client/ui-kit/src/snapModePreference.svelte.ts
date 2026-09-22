import type { SnapMode } from "@shadowcat/render";

/**
 * The session's snap-MODE preference — which lattice point a snapping tool's point resolves
 * to (`"center"`: the cell's center; `"vertex"`: the nearest cell corner, see `SnapMode`).
 * Client-local and session-scoped: never written to a document and never broadcast, because
 * it only steers how THIS client's tool clicks resolve — the wall/region geometry a tool
 * stores is plain scene-unit floats regardless of the mode used to pick the points.
 *
 * A stable instance is held by the shell and shared via AppContext: `ToolRail`'s snap-mode
 * toggle writes it, and `Stage` pushes it into the render engine through
 * `SceneToolHost.setSnapMode` (both on toggle and on engine (re)creation, so a rebuilt engine
 * inherits the session's mode). Sibling of `SceneSelection`/`SpeakAsToken`: the same
 * stable-instance/mutate-in-place `$state` shape, and likewise orthogonal to the document
 * stream — no rollback or conflict handling applies to it.
 */
export class SnapModePreference {
  /** Backing store for {@link SnapModePreference.mode}. */
  #mode = $state<SnapMode>("center");

  /** The current snap mode. `"center"` (the default) resolves a snap to the cell's center,
   * preserving the behavior every existing scene's authoring flow was built around;
   * `"vertex"` resolves to the nearest cell corner.
   * @returns The current snap mode. */
  get mode(): SnapMode {
    return this.#mode;
  }

  /** Set the snap mode. `Stage` observes the change reactively and re-pushes it to the
   * engine's snap chokepoint, so the writer never touches the engine itself.
   * @param mode - The snap mode to apply.
   * @example snapModePreference.set("vertex");
   */
  set(mode: SnapMode): void {
    this.#mode = mode;
  }
}
