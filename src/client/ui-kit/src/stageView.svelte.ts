import type { CameraTransform, Point } from "@shadowcat/render";

/** Answers whether the fog the canvas currently paints reveals a token's position. */
type RevealProbe = (tokenId: string, at: Point) => boolean;

/**
 * The stage camera and fog as reactive state, so DOM drawn over the canvas (a stage overlay) can
 * stay anchored to scene points while the camera pans and zooms, and never show a position the
 * canvas hides. `Stage` writes the camera from the render engine's applied transform and attaches
 * the engine's fog probe; an overlay reads `sceneToScreen` and `isTokenRevealed`, each of which
 * depends on `$state` and so re-runs its caller on every camera or mask change.
 *
 * The mapping is the camera's own: `screen = scene * scale + offset`, identically on both
 * axes (`Camera.sceneToScreen`), in pixels relative to the canvas — which `.stage-overlays`
 * shares as its origin. A stable instance is held by the shell and shared via AppContext, like
 * `SnapModePreference`; before the first transform it is the identity, and before a probe is
 * attached nothing is revealed (fog is the sole client-side secrecy gate, so the default is
 * closed).
 */
export class StageView {
  /** Backing store for {@link StageView.transform}. */
  #transform = $state<CameraTransform>({ x: 0, y: 0, scale: 1 });
  /** Bumped on every applied fog mask so a reader of `isTokenRevealed` re-runs. */
  #maskRevision = $state(0);
  /** The attached engine's fog probe, or `null` when no engine is mounted. */
  #probe: RevealProbe | null = null;

  /** The last camera transform `Stage` applied; the identity before the first one.
   * @returns The current `{x, y, scale}` offset/scale pair. */
  get transform(): CameraTransform {
    return this.#transform;
  }

  /** Record the camera transform the render engine just applied.
   * @param transform - The applied transform.
   * @example stageView.set({ x: 40, y: -10, scale: 2 });
   */
  set(transform: CameraTransform): void {
    this.#transform = { x: transform.x, y: transform.y, scale: transform.scale };
  }

  /** Project a scene point to canvas-relative screen pixels with the current camera.
   * Reactive: reading it inside a `$derived`/`$effect` tracks the camera.
   * @param p - A scene-coordinate point.
   * @returns The point's position in pixels from the canvas's top-left corner.
   * @example stageView.sceneToScreen({ x: 100, y: 50 });
   */
  sceneToScreen(p: Point): Point {
    const t = this.#transform;
    return { x: p.x * t.scale + t.x, y: p.y * t.scale + t.y };
  }

  /** Attach the live engine's fog probe. A later attach replaces the current probe; the returned
   * detach only clears the probe if it is still the current one, so an out-of-order teardown
   * cannot clear a newer engine's probe.
   * @param probe - Answers whether the applied fog reveals a token at a scene point.
   * @returns A detach function; safe to call multiple times.
   * @example const detach = stageView.attachReveal((id, at) => engine.isTokenRevealed(id, at));
   */
  attachReveal(probe: RevealProbe): () => void {
    this.#probe = probe;
    this.#maskRevision += 1;
    return () => {
      if (this.#probe === probe) {
        this.#probe = null;
        this.#maskRevision += 1;
      }
    };
  }

  /** Announce that the engine applied a new fog mask, re-running every reactive reader of
   * {@link StageView.isTokenRevealed}.
   * @example stageView.noteMaskApplied();
   */
  noteMaskApplied(): void {
    this.#maskRevision += 1;
  }

  /** Whether the fog the canvas paints reveals a token at `at`. Reactive on mask changes.
   * Fail-closed: `false` while no engine is attached.
   * @param tokenId - The token's document id.
   * @param at - The token's scene-coordinate position.
   * @returns `true` when a viewer currently sees that position.
   * @example stageView.isTokenRevealed("t1", { x: 150, y: 150 });
   */
  isTokenRevealed(tokenId: string, at: Point): boolean {
    // Reading the revision registers the reactive dependency; its value is unused.
    void this.#maskRevision;
    return this.#probe ? this.#probe(tokenId, at) : false;
  }
}
