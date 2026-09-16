<script lang="ts">
  // A `Teleport` trigger's destination editor: scene picker (live search), x/y number inputs,
  // pick-on-stage targeting (`ToolController.beginPickPortalTarget`/`endPickPortalTarget`), an
  // elevation input, and a VFX asset picker. `target` is `$bindable`: it lives inside the
  // `$state`-proxied `ToolController.regionTriggers` tree owned by `ToolRail.svelte`, and a
  // plain (non-bindable) prop mutated from a CHILD component is a dev-mode ownership violation
  // in Svelte 5 (`ownership_invalid_mutation`) — `bind:target` at the call site is the sanctioned
  // two-way seam across that component boundary.
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { PortalTarget, WireDocument, WireSearchHit, SubscriptionHandle } from "@shadowcat/core";
  import type { ToolController } from "./controller.svelte";

  let { target = $bindable(), row, controller }: {
    /** The teleport trigger's destination; mutated through the two-way binding. */
    target: PortalTarget;
    /** This row's index into `controller.regionTriggers`, for pick-on-stage targeting. */
    row: number;
    /** The shared tool-controller instance (pick-on-stage state + trigger). */
    controller: ToolController;
  } = $props();

  const ctx = getAppContext();
  const { t } = ctx;

  /** Destination scene search text; empty renders the currently-authored destination as a
   * placeholder rather than a result list. */
  let query = $state("");
  let hits = $state<WireDocument[]>([]);

  // Live FTS search over scenes, mirroring `ActorsPanel`'s live-search effect: torn down and
  // recreated on every query change, cancel-guarded against a stale query's callback firing
  // after a newer one has already re-subscribed.
  $effect(() => {
    const q = query.trim();
    if (!q) {
      hits = [];
      return;
    }
    let handle: SubscriptionHandle | null = null;
    let cancelled = false;
    void ctx
      .searchDocuments(q, { limit: 20, docTypes: ["scene"] }, (h: WireSearchHit[]) => {
        if (cancelled) return;
        hits = h.map((x) => x.document);
      })
      .then((h) => {
        if (cancelled) h.unsubscribe();
        else handle = h;
      })
      .catch(() => {
        /* no transport: leave last hits, re-subscribe on next keystroke */
      });
    return () => {
      cancelled = true;
      handle?.unsubscribe();
    };
  });

  const destinationLabel = $derived(
    target.scene ? (ctx.documents.get(target.scene)?.name ?? target.scene) : t("tools.triggerTeleportScene"),
  );

  /** Set the trigger's destination scene from a search hit and clear the search UI.
   * @param id The picked scene document's id.
   * @example
   * ```
   * declare const editor: { pickScene(id: string): void };
   * editor.pickScene("scene-1");
   * ```
   */
  function pickScene(id: string): void {
    target.scene = id;
    query = "";
    hits = [];
  }

  /** Parse the elevation input: empty clears to `null` (leave the token's current elevation
   * unchanged, per `PortalTarget.elevation`'s own null-means-unchanged semantics); otherwise a
   * finite number, mirroring `parseElevation`'s null-normalization.
   * @param raw The input's raw string value.
   * @example
   * ```
   * declare const editor: { editElevation(raw: string): void };
   * editor.editElevation("10");
   * ```
   */
  function editElevation(raw: string): void {
    const trimmed = raw.trim();
    if (trimmed === "") {
      target.elevation = null;
      return;
    }
    const n = Number(trimmed);
    if (Number.isFinite(n)) target.elevation = n;
  }
</script>

<div class="trigger-teleport">
  <input
    data-testid="region-trigger-teleport-scene"
    aria-label={t("tools.triggerTeleportScene")}
    bind:value={query}
    placeholder={destinationLabel}
  />
  {#if hits.length > 0}
    <ul class="scene-hits">
      {#each hits as h (h.id)}
        <li>
          <button type="button" data-testid="region-trigger-teleport-scene-hit" onclick={() => pickScene(h.id)}>
            {h.name}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  <input
    type="number"
    data-testid="region-trigger-teleport-x"
    aria-label={t("tools.triggerTeleportX")}
    value={target.x}
    onchange={(e) => {
      const n = Number(e.currentTarget.value);
      if (Number.isFinite(n)) target.x = n;
    }}
  />
  <input
    type="number"
    data-testid="region-trigger-teleport-y"
    aria-label={t("tools.triggerTeleportY")}
    value={target.y}
    onchange={(e) => {
      const n = Number(e.currentTarget.value);
      if (Number.isFinite(n)) target.y = n;
    }}
  />
  <button
    type="button"
    data-testid="region-trigger-teleport-pick"
    onclick={() => controller.beginPickPortalTarget(row)}
  >
    {controller.pickingPortalRow === row ? t("tools.triggerTeleportPicking") : t("tools.triggerTeleportPick")}
  </button>
  <input
    type="number"
    data-testid="region-trigger-teleport-elevation"
    aria-label={t("tools.triggerTeleportElevation")}
    value={target.elevation ?? ""}
    onchange={(e) => editElevation(e.currentTarget.value)}
  />
  <button
    type="button"
    data-testid="region-trigger-teleport-vfx"
    onclick={() =>
      void ctx.pickAsset({ kind: "image", tags: ["vfx"] }).then((id) => {
        if (id !== null) target.vfx = id;
      })}
  >
    {target.vfx ? `${t("tools.triggerTeleportVfx")}: ${target.vfx}` : t("tools.triggerTeleportVfx")}
  </button>
  {#if target.vfx}
    <button
      type="button"
      data-testid="region-trigger-teleport-vfx-clear"
      onclick={() => (target.vfx = null)}
    >
      {t("tools.triggerTeleportVfxClear")}
    </button>
  {/if}
</div>

<style lang="scss">
  .trigger-teleport {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
  }
  .scene-hits {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .scene-hits button {
    width: 100%;
    text-align: left;
  }
</style>
