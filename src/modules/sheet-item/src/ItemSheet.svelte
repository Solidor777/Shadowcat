<script lang="ts">
  import { getAppContext, SystemTreeEditor, setField } from "@shadowcat/ui-kit";
  import { createSubscriber } from "svelte/reactivity";
  import { getPointer, isDiceNotation, type WireDocument, type ItemSystem } from "@shadowcat/core";

  // Item sheet: `name` control (resolved via `namePrefix`, sibling of `systemPrefix` — see
  // below) + dice-notation string values get a roll-to-chat affordance (posts `/roll <formula>`
  // on the default "general" channel over the chat wire — the server executes it) + the
  // `system` tree editor. `buildItemDoc`'s contract puts an item's real display
  // name alongside `system`, same as every other doc_type — `system` carries only the opaque,
  // genuinely game-system-owned fields. Reads the OPTIMISTIC store; edits use the RAW current
  // value as the OCC pre-image.
  let {
    docId,
    systemPrefix,
    close,
  }: {
    /** The document that owns `systemPrefix`'s tree — the item's own id for a top-level
     * (linked) item, or the parent actor's id for an item embedded in its inventory. */
    docId: string;
    /** The write root for the opaque `system` tree; `namePrefix` (below) is derived from
     * it by stripping the trailing `/system`, since it may be nested under an actor. */
    systemPrefix: string;
    /** Closes the hosting panel; wired to the header close button. */
    close: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // `systemPrefix` (from `resolveDocRef`'s `writePrefix`) always ends in `/system` — for a
  // top-level (linked) item that's `/system`; for an embedded item (opened from an actor's
  // inventory) it's `/embedded/item/<i>/system`, and `docId` resolves to the PARENT document's
  // id either way. The envelope `name` band lives at the SAME node as `system`, so `basePrefix`
  // (the prefix with the trailing `/system` stripped) is the sibling root for `/name` — reading
  // or writing the literal `/name` for an embedded item would hit the PARENT ACTOR's own name.
  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const namePrefix = $derived(`${basePrefix}/name`);

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
  const name = $derived.by((): string | null => (doc ? (getPointer(doc, namePrefix) as string | null | undefined) ?? null : null));
  const system = $derived.by((): ItemSystem | undefined => (doc ? (getPointer(doc, systemPrefix) as ItemSystem | undefined) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  // Dice-notation leaves (string values that look like `NdM`), for the roll affordance.
  const rollable = $derived.by((): {
    /** The `system` field name this formula came from; the roll button's label. */
    key: string;
    /** The trimmed dice-notation string, passed verbatim to `roll` on click. */
    formula: string;
  }[] => {
    if (!system) return [];
    return Object.entries(system)
      .filter(([, v]) => typeof v === "string" && isDiceNotation(v))
      .map(([key, v]) => ({ key, formula: (v as string).trim() }));
  });

  /** Update the `name` field at `namePrefix` (the sibling of `systemPrefix`, correctly
   * embedded-aware — distinct from the opaque `system` tree, which the `SystemTreeEditor`
   * below edits directly via its own setField calls).
   * @param value The new name.
   * @example
   * ```
   * // private function; not part of the public API — wired to the name
   * // input's onchange above
   * setName("Longsword +1");
   * ```
   */
  function setName(value: string): void {
    if (!doc) return;
    setField(ctx, docId, namePrefix, name, value);
  }

  /** Posts `formula` to chat as a `/roll` command on the hardcoded `"general"`
   * channel. `channel` is a purely client-chosen display label the server
   * never validates or derives audience from (see `Audience`'s doc comment;
   * `handle_send_message`'s channel checks only check non-empty/length).
   * Posting to `"general"` before a GM has ever added it to the channel
   * registry is harmless: the message still sends, and any UI resolving the
   * channel's display name falls back to the raw id for an unregistered one
   * (mirrors `ChatPanel`'s `channelDisplayName`).
   * @param formula The dice-notation string to roll (already filtered through
   * `isDiceNotation`; see `rollable` above).
   * @example
   * ```
   * // private function; not part of the public API — wired to each
   * // rollable button's onclick below
   * roll("2d6+3");
   * ```
   */
  function roll(formula: string): void {
    ctx.chat.send({ channel: "general", content: `/roll ${formula}` });
  }
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{name ?? t("sheetItem.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>

  {#if doc && system}
    <label>{t("sheetItem.name")}
      <input aria-label={t("sheetItem.name")} value={name ?? ""} disabled={readOnly}
        onchange={(e) => setName((e.currentTarget as HTMLInputElement).value)} /></label>

    {#if rollable.length > 0}
      <div class="rolls">
        {#each rollable as r (r.key)}
          <button type="button" onclick={() => roll(r.formula)}>{r.key}: {r.formula}</button>
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
