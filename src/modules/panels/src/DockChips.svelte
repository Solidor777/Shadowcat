<script lang="ts">
  import type { PanelMeta } from "@shadowcat/core";
  import { getAppContext } from "@shadowcat/ui-kit";

  /** Renders minimized panel ids as labeled restore chips. Trusts `minimized`/
   * `meta` to already be gmOnly-filtered by the caller (PanelHost is the one
   * place that filters registrations by role) — this component applies no
   * additional role logic. */
  let {
    minimized,
    meta,
    onRestore,
  }: {
    minimized: readonly string[];
    meta: ReadonlyMap<string, PanelMeta>;
    onRestore: (id: string) => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;
</script>

<div class="dock-chips" role="toolbar" aria-label={t("panels.moreViews")}>
  {#each minimized as id (id)}
    {@const m = meta.get(id)}
    <button
      type="button"
      class="chip"
      data-testid="chip-{id}"
      aria-label="{t('panels.restore')} {m ? t(m.labelKey) : id}"
      title="{t('panels.restore')} {m ? t(m.labelKey) : id}"
      onclick={() => onRestore(id)}
    >{m?.icon ?? ""} {m ? t(m.labelKey) : id}</button>
  {/each}
</div>

<style lang="scss">
  .dock-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    padding: 0.25rem;
  }
  .chip {
    /* Touch target floor (mobile invariant). */
    min-height: 44px;
    padding: 0 0.75rem;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-overlay);
    color: var(--text-primary);
    font-size: 0.85rem;
    cursor: pointer;
  }
</style>
