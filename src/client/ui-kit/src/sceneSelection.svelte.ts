// The scene the game-settings per-scene section edits (M12d "Configure"). A stable instance
// created by the shell and shared via AppContext: the scene browser sets it, GameSettingsPanel
// reads it to preset its picker. Distinct from `activeScene` (global render target) and
// `gmViewedScene` (GM local camera) — configuring a scene never moves any camera. Reactive
// ($state) + mutated in place (never reassigned) so the AppContext-captured reference stays valid.
export class SceneSelection {
  #id = $state<string | null>(null);

  get configureSceneId(): string | null {
    return this.#id;
  }

  select(id: string | null): void {
    this.#id = id;
  }
}
