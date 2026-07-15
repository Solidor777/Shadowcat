<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, SystemTreeEditor } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";

  // The always-available sheet: document envelope metadata + the type-aware tree editor
  // over the writable `system` body. Reads the OPTIMISTIC store so redaction is free.
  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive subscription: `ctx.documents` (OptimisticClient) is a plain-callback store, not a
  // Svelte rune — a $derived that reads it without this bridge freezes at first render. Mirrors
  // GameSettingsPanel's `ws`/`wsys` pattern: subscribe() is called only in the `doc` derived (the
  // sole direct `ctx.documents` read); `system`/`readOnly` derive from `doc` and re-derive with it.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const system = $derived.by((): unknown => (doc ? getPointer(doc, systemPrefix) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{t("sheetFallback.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>
  {#if doc}
    <dl class="envelope">
      <dt>{t("sheetFallback.type")}</dt><dd>{doc.doc_type}</dd>
      <dt>{t("sheetFallback.id")}</dt><dd class="mono">{doc.id}</dd>
      <dt>{t("sheetFallback.owner")}</dt><dd class="mono">{doc.owner ?? "—"}</dd>
    </dl>
    <h3>{t("sheetFallback.system")}</h3>
    <SystemTreeEditor {doc} basePath={systemPrefix} root={system} {readOnly} />
  {:else}
    <p class="missing">{t("sheets.missing")}</p>
  {/if}
</div>

<style lang="scss">
  .sheet { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); height: 100%; overflow: auto; }
  .sheet-header { display: flex; align-items: center; justify-content: space-between; }
  .close { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .close:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .envelope { display: grid; grid-template-columns: auto 1fr; gap: var(--space-1); margin: 0; }
  dt { font-weight: 700; opacity: 0.7; }
  .mono { font-family: monospace; }
  .missing { opacity: 0.7; font-style: italic; }
</style>
