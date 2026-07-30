<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { abilityMod, evalFormula } from "./rules";

  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx = getAppContext();
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  // #region sheet-read
  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const ATTRS = ["str", "dex", "con"] as const;
  /** Current attribute score from the opaque system band (default 10). */
  function score(attr: string): number {
    const v = doc ? getPointer(doc, `${systemPrefix}/attributes/${attr}`) : undefined;
    return typeof v === "number" ? v : 10;
  }
  const power = $derived.by((): number | null => {
    subscribe();
    return doc ? evalFormula("attributes.str + attributes.con", getPointer(doc, systemPrefix)) : null;
  });
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));
  // #endregion sheet-read

  // #region sheet-write
  /** Writes one attribute with its OCC pre-image (raw current stored value). */
  function setScore(attr: string, value: number): void {
    if (!doc) return;
    const path = `${systemPrefix}/attributes/${attr}`;
    setField(ctx, docId, path, getPointer(doc, path), value);
  }
  // #endregion sheet-write
</script>

<div class="sheet" role="dialog" aria-label="Character sheet">
  <header>
    <h2>{doc?.name ?? "Character"}</h2>
    <button type="button" aria-label="Close" onclick={close}>×</button>
  </header>
  {#if doc}
    {#each ATTRS as attr (attr)}
      <label>
        {attr.toUpperCase()}
        <input
          type="number"
          value={score(attr)}
          disabled={readOnly}
          onchange={(e) => setScore(attr, Number(e.currentTarget.value))}
        />
        <span>mod {abilityMod(score(attr))}</span>
      </label>
    {/each}
    <p>Power (str + con): {power ?? "—"}</p>
  {/if}
</div>

<style>
  .sheet { padding: 0.5rem; }
  input { min-height: 44px; }
  button { min-height: 44px; min-width: 44px; }
</style>
