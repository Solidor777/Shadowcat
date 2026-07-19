<script lang="ts">
  // Host-rendered template chrome for any doc_type's sheet (§6.1). Reads provenance/instances
  // from the templates seam; shows a source badge + pull/revert (stamped, authorized) and push
  // (has instances, authorized). The module sheet body never opts in.
  import { getAppContext } from "./appContext";

  let { docId }: { docId: string } = $props();
  const ctx = getAppContext();
  const t = ctx.t;

  const doc = $derived(ctx.documents.get(docId));
  const template = $derived(doc?.source ? ctx.documents.get(doc.source.id) : undefined);
  const sync = $derived(ctx.templates.syncState(docId));
  const canPull = $derived(ctx.templates.canPull(docId));
  const canPush = $derived(ctx.templates.canPush(docId));
  const hasSource = $derived(!!doc?.source && !!template);
</script>

{#if hasSource || canPush}
  <div class="template-controls">
    {#if hasSource}
      <span class="badge" class:changed={sync === "template_changed"}>
        {t("templates.badge.source", { name: template?.name ?? "" })}
        <span class="state">{sync === "template_changed" ? t("templates.badge.changed") : t("templates.badge.upToDate")}</span>
      </span>
      {#if canPull}
        <button type="button" onclick={() => ctx.templates.pull(docId)}>{t("templates.action.pull")}</button>
        <button type="button" onclick={() => ctx.templates.revert(docId)}>{t("templates.action.revert")}</button>
      {/if}
    {/if}
    {#if canPush}
      <button type="button" onclick={() => ctx.templates.push(docId)}>{t("templates.action.push")}</button>
    {/if}
  </div>
{/if}

<style lang="scss">
  .template-controls { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); padding: var(--space-1) var(--space-2); border-bottom: 1px solid var(--border); background: var(--surface); }
  .badge { display: inline-flex; align-items: center; gap: var(--space-1); font-size: var(--font-sm); opacity: 0.85; }
  .badge.changed .state { color: var(--accent); font-weight: 600; }
  button { min-height: 44px; padding: 0 var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); color: inherit; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
