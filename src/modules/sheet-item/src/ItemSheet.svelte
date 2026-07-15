<script lang="ts">
  import { getAppContext, SystemTreeEditor, setField } from "@shadowcat/ui-kit";
  import { createSubscriber } from "svelte/reactivity";
  import { getPointer, isDiceNotation, type WireDocument, type ItemSystem } from "@shadowcat/core";

  // Item sheet: name control + dice-notation string values get a roll-to-chat affordance
  // (posts `/roll <formula>` on the default "general" channel over the M11 chat wire — the
  // server executes it) + the `system` tree editor. Reads the OPTIMISTIC store; edits use
  // the RAW current value as the OCC pre-image.
  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive subscription: ctx.documents is a plain-callback store, not a rune — a $derived
  // reading it directly freezes at first read and never observes later edits, corrupting
  // compound-field OCC pre-images on a second edit. Mirrors
  // GameSettingsPanel/FactionsPanel/ConditionsPanel: one subscribe() call, invoked as the
  // first statement of every $derived reading ctx.documents.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const system = $derived.by((): ItemSystem | undefined => (doc ? (getPointer(doc, systemPrefix) as ItemSystem | undefined) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  // Dice-notation leaves (string values that look like `NdM`), for the roll affordance.
  const rollable = $derived.by((): { key: string; formula: string }[] => {
    if (!system) return [];
    return Object.entries(system)
      .filter(([, v]) => typeof v === "string" && isDiceNotation(v))
      .map(([key, v]) => ({ key, formula: (v as string).trim() }));
  });

  function set(field: string, value: unknown): void {
    if (!doc) return;
    const path = `${systemPrefix}/${field}`;
    setField(ctx, docId, path, getPointer(doc, path), value);
  }

  function roll(formula: string): void {
    ctx.chat.send({ channel: "general", content: `/roll ${formula}` });
  }
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{system?.name ?? t("sheetItem.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>

  {#if doc && system}
    <label>{t("sheetItem.name")}
      <input aria-label="sheetItem.name" value={system.name ?? ""} disabled={readOnly}
        onchange={(e) => set("name", (e.currentTarget as HTMLInputElement).value)} /></label>

    {#if rollable.length > 0}
      <div class="rolls">
        {#each rollable as r (r.key)}
          <button type="button" aria-label="sheetItem.roll" onclick={() => roll(r.formula)}>{r.key}: {r.formula}</button>
        {/each}
      </div>
    {/if}

    <details>
      <summary>{t("sheetItem.system")}</summary>
      <SystemTreeEditor {doc} basePath={systemPrefix} root={system} {readOnly} />
    </details>
  {:else}
    <p class="missing">{t("sheets.missing")}</p>
  {/if}
</div>

<style lang="scss">
  .sheet { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); height: 100%; overflow: auto; }
  .sheet-header { display: flex; align-items: center; justify-content: space-between; }
  .close { min-width: 44px; min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .close:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  label { display: flex; flex-direction: column; gap: 2px; }
  .rolls { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .rolls button { min-height: 44px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); font-family: monospace; }
  .rolls button:focus-visible { outline: 2px solid var(--accent); }
  .missing { opacity: 0.7; font-style: italic; }
</style>
