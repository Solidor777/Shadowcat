/**
 * The scene the game-settings per-scene "Configure" section edits. A stable instance
 * created by the shell and shared via AppContext: the scene browser sets it, GameSettingsPanel
 * reads it to preset its picker. Distinct from `activeScene` (global render target) and
 * `gmViewedScene` (GM local camera) — configuring a scene never moves any camera. Reactive
 * (`$state`) + mutated in place (never reassigned) so the AppContext-captured reference stays valid.
 *
 * Sibling of `ActorSelection`/`TokenSelection`: same stable-instance/mutate-in-place shape, and
 * likewise does not prune when the referenced scene is later deleted — `GameSettingsPanel` must
 * handle a `configureSceneId` that no longer resolves. Shares `ActorSelection.select`'s
 * repeat-selection behavior (a no-op for reactivity, `$state`'s default `===` equality), NOT
 * `TokenSelection.set`'s behavior of re-triggering on any call that starts non-empty.
 */
export class SceneSelection {
  /** Backing store for {@link SceneSelection.configureSceneId}. */
  #id = $state<string | null>(null);

  /** The scene id currently targeted for configuration, or `null` when none is selected.
   * @returns The configure-target scene id, or `null`. */
  get configureSceneId(): string | null {
    return this.#id;
  }

  /** Set (or clear, with `null`) the configure-target scene. Re-selecting the CURRENTLY
   * selected id is a no-op for reactivity (see the class doc).
   * @param id - The scene id to target, or `null` to clear.
   * @example sceneSelection.select("scene-1");
   */
  select(id: string | null): void {
    this.#id = id;
  }
}
