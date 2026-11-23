<script lang="ts">
  // The viewed scene's dormant regions: anchored regions whose carrier token no longer resolves.
  // A dormant region draws and hit-tests nothing (`resolveRegionGeometry` returns `null`), so the
  // stage can never pick it; this list is the GM's only handle on it. The dormancy test is
  // `carrierPoseFor`, the same rule the render view and the server use. Each row offers the
  // region editor (`ToolController.editingEntity`), a re-anchor to the single selected token
  // (`ToolController.setRegionAnchor`, which keeps the stored offsets) and a delete.
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { carrierPoseFor, type RegionEngine, type WireDocument } from "@shadowcat/core";
  import type { ToolController } from "./controller.svelte";

  let { controller }: {
    /** The shared tool-controller instance (the region editor selection and the anchor write). */
    controller: ToolController;
  } = $props();

  const ctx = getAppContext();
  const { t } = ctx;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  /** The regions on the viewed scene whose anchor names a carrier that does not resolve.
   * @returns The dormant region documents, in document order.
   * @example
   * ```
   * dormantRegions(); // [] when every anchored region's carrier exists
   * ```
   */
  function dormantRegions(): WireDocument[] {
    const sceneId = ctx.viewedSceneId;
    if (sceneId === null) return [];
    return ctx.documents.query("region").filter((region) => {
      if (region.parent_id !== sceneId) return false;
      const anchor = (region.engine as RegionEngine | undefined)?.anchor;
      return !!anchor && carrierPoseFor(region, ctx.documents.get(anchor.token)) === null;
    });
  }

  const dormant = $derived.by((): WireDocument[] => {
    subscribe();
    return dormantRegions();
  });

  /** The one selected token that can carry `region` (a token on the region's scene), or `null`
   * when the selection is empty, has several tokens, or the token is not on that scene.
   * @param region The dormant region.
   * @returns The selected token's id, or `null`.
   * @example
   * ```
   * declare const region: WireDocument;
   * selectedCarrier(region); // "token-1" or null
   * ```
   */
  function selectedCarrier(region: WireDocument): string | null {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size !== 1) return null;
    const [id] = ids;
    return carrierPoseFor(region, ctx.documents.get(id)) ? id : null;
  }

  /** Short row text: the region's behavior and a short id.
   * @param region The dormant region.
   * @returns The visible label.
   * @example
   * ```
   * declare const region: WireDocument;
   * rowLabel(region); // "arrest (a1b2c3d4)"
   * ```
   */
  function rowLabel(region: WireDocument): string {
    const eng = region.engine as RegionEngine;
    return t("tools.regionDormantRow", { behavior: eng.behavior, id: region.id.slice(0, 8) });
  }

  /** Open the region in the rail's region editor.
   * @param region The dormant region.
   * @example
   * ```
   * declare const region: WireDocument;
   * editRegion(region);
   * ```
   */
  function editRegion(region: WireDocument): void {
    controller.editingEntity = { kind: "region", id: region.id };
  }

  /** Re-anchor the region to the selected token, keeping its anchor flags.
   * @param region The dormant region.
   * @example
   * ```
   * declare const region: WireDocument;
   * reanchor(region);
   * ```
   */
  function reanchor(region: WireDocument): void {
    const token = selectedCarrier(region);
    const anchor = (region.engine as RegionEngine).anchor;
    if (!token || !anchor) return;
    controller.setRegionAnchor(region.id, { ...anchor, token });
  }

  /** Delete the region (full pre-image op); closes its editor when it was open.
   * @param region The dormant region.
   * @example
   * ```
   * declare const region: WireDocument;
   * removeRegion(region);
   * ```
   */
  function removeRegion(region: WireDocument): void {
    ctx.dispatchIntent([{ op: "delete", doc: region }]);
    if (controller.editingEntity?.id === region.id) controller.editingEntity = null;
  }
</script>

{#if dormant.length > 0}
  <div class="dormant" data-testid="region-dormant-list">
    <span class="title">{t("tools.regionDormantTitle")}</span>
    {#each dormant as region (region.id)}
      <div class="row" data-testid="region-dormant-row" data-region-id={region.id}>
        <span class="label">{rowLabel(region)}</span>
        <button type="button" class="act" data-testid="region-dormant-edit" onclick={() => editRegion(region)}>{t("tools.regionDormantEdit")}</button>
        <button
          type="button"
          class="act"
          data-testid="region-dormant-reanchor"
          disabled={selectedCarrier(region) === null}
          onclick={() => reanchor(region)}
        >{t("tools.regionDormantReanchor")}</button>
        <button type="button" class="act" data-testid="region-dormant-delete" onclick={() => removeRegion(region)}>{t("tools.delete")}</button>
      </div>
    {/each}
    <span class="hint">{t("tools.regionDormantHint")}</span>
  </div>
{/if}

<style lang="scss">
  .dormant {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .title {
    font-weight: 600;
  }
  .hint {
    color: var(--text-secondary);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }
  .label {
    flex: 1 1 100%;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .act {
    min-height: var(--input-height-coarse);
    min-width: var(--input-height-coarse);
    max-width: 100%;
    box-sizing: border-box;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .act:disabled {
    cursor: not-allowed;
    opacity: 0.6;
  }
  .act:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
