<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { abilityMod, evalFormula } from "./rules";

  let {
    docId,
    systemPrefix,
    close,
  }: {
    /** The actor document this sheet edits — the `setField`/`ctx.documents.get` key. */
    docId: string;
    /** The write root within `docId` for this sheet's fields (e.g. `/system` for a top-level
     * actor); every `getPointer`/`setField` call below is relative to it. */
    systemPrefix: string;
    /** Closes the hosting panel; wired to this sheet's header close button. */
    close: () => void;
  } = $props();

  const ctx = getAppContext();
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  // #region sheet-read
  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const ATTRS = ["str", "dex", "con"] as const;
  /** Current attribute score from the opaque system band (default 10 when
   * unset or non-numeric — degenerate sheet data must not crash the sheet).
   * @param attr - The attribute key (e.g. `"str"`).
   * @returns The stored numeric score, or `10` if unset or non-numeric.
   * @example
   * ```
   * // private function; not part of the public API — invoked from this
   * // component's template and the `power` derived below
   * score("str");
   * ```
   */
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
  /** Writes one attribute via `setField`'s OCC contract — see
   * `setField` for the pre-image invariant this
   * call must satisfy; not restated here to avoid a second, driftable copy.
   * @param attr - The attribute key (e.g. `"str"`).
   * @param value - The new numeric score to write.
   * @example
   * ```
   * // private function; not part of the public API — invoked only from this
   * // component's number-input onchange handler below
   * setScore("str", 14);
   * ```
   */
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
