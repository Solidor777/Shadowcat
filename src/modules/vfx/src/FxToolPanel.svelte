<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { SCENE_TOOL_CONTRACT } from "@shadowcat/core";
  import { fxToolState } from "./fxToolState.svelte";
  import { onFxSceneClick } from "./onSceneClick";

  const ctx = getAppContext();
  const t = ctx.t;

  // Registers the FX scene tool for the life of this component. PanelHost mounts every
  // PANEL_CONTRACT registration unconditionally (launcher-closed is a layout state, not a
  // mount state), so this effect runs from module install onward regardless of whether a
  // user has ever opened this panel.
  $effect(() => {
    return ctx.contributions.contribute({
      id: "vfx:scene-tool",
      contract: SCENE_TOOL_CONTRACT,
      component: null,
      sceneTool: {
        id: "vfx",
        icon: "✨",
        labelKey: "vfx.toolLabel",
        onSceneClick: (x, y) => {
          void onFxSceneClick(ctx, x, y);
        },
      },
    });
  });

  /** Open the asset picker (VFX-tagged) and store the pick.
   * @example
   * ```
   * // private function; wired to the Pick effect button below
   * void pickEffect();
   * ```
   */
  async function pickEffect(): Promise<void> {
    const id = await ctx.pickAsset({ kind: "image", tags: ["vfx"] });
    if (id) fxToolState.assetId = id;
  }

  /** Opens the asset picker for a paired sound; the picker filters to the asset browser's
   * audio kind. Stores or clears the pick.
   * @example
   * ```
   * // private function; wired to the Pick sound button below
   * void pickSound();
   * ```
   */
  async function pickSound(): Promise<void> {
    const id = await ctx.pickAsset({ kind: "audio" });
    if (id) fxToolState.soundId = id;
  }
</script>

<div class="fx-panel">
  <div class="fx-row">
    <button type="button" data-testid="fx-pick-effect" onclick={pickEffect}>{t("vfx.pickEffect")}</button>
    {#if fxToolState.assetId}
      <img class="fx-preview" src={ctx.assets.url(fxToolState.assetId)} alt="" data-testid="fx-preview" />
    {/if}
  </div>
  <label class="fx-row">
    {t("vfx.scale")}
    <input
      type="number"
      min="0.1"
      max="8"
      step="0.1"
      data-testid="fx-scale"
      aria-label={t("vfx.scale")}
      bind:value={fxToolState.scale}
    />
  </label>
  <div class="fx-row">
    <button type="button" data-testid="fx-pick-sound" onclick={pickSound}>{t("vfx.pickSound")}</button>
    {#if fxToolState.soundId}
      <button type="button" data-testid="fx-clear-sound" onclick={() => (fxToolState.soundId = null)}>
        {t("vfx.clearSound")}
      </button>
    {/if}
  </div>
</div>

<style lang="scss">
  .fx-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
  }
  .fx-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .fx-preview {
    max-width: 48px;
    max-height: 48px;
  }
</style>
