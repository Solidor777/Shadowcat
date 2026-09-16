// Module-scoped persistent config for the FX scene tool: the last-picked asset/scale/sound,
// shared between the config panel (FxToolPanel.svelte) and the scene tool's click handler —
// a user who never opens the panel still gets an inline asset-pick prompt on first click
// (see FxToolPanel/the scene-tool registration's own doc).
/** The FX tool's persistent, module-scoped configuration. One instance, module-wide (not
 * per-component) — the config panel and the scene-tool click handler must observe the SAME
 * state regardless of which one last changed it. */
class FxToolState {
  /** The currently-picked effect asset id, or `null` if never picked. */
  assetId = $state<string | null>(null);
  /** Uniform scale multiplier for the next placement. */
  scale = $state(1);
  /** The currently-picked paired sound asset id, or `null` for none. */
  soundId = $state<string | null>(null);
}

/** The one shared `FxToolState` instance for this module. */
export const fxToolState = new FxToolState();
