<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, SystemTreeEditor, setField } from "@shadowcat/ui-kit";
  import { getPointer, actorDisplayName, type WireDocument, type ActorEngine, type FactionRegistryEngine } from "@shadowcat/core";

  // Actor sheet: envelope `name` + engine-known fields (displayName, faction, shape, size)
  // as real controls + the opaque `system` body as a tree editor + the embedded-items
  // inventory (opens each via openDocument). Reads the OPTIMISTIC store (per-recipient
  // redaction + OwnerOrGm naming are free). Every edit's `old` is the RAW current stored
  // value. `systemPrefix` (from `resolveDocRef`'s `writePrefix`) always ends in `/system`
  // (top-level `/system`, or `/embedded/actor/0/system` for an instanced token) — the
  // engine/name bands live at the SAME node, so `basePrefix` (the prefix with the trailing
  // `/system` stripped) is the sibling root for `/engine` and `/name`.
  let {
    docId,
    systemPrefix,
    close,
  }: {
    /** The actor document (or its parent token, for an instanced actor) this sheet edits. */
    docId: string;
    /** The write root for the opaque `system` tree; `basePrefix`/`enginePrefix`/`namePrefix`
     * below are all derived from it — see the module-level comment. */
    systemPrefix: string;
    /** Closes the hosting panel; wired to the header close button. */
    close: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const basePrefix = $derived(systemPrefix.replace(/\/system$/, ""));
  const enginePrefix = $derived(`${basePrefix}/engine`);
  const namePrefix = $derived(`${basePrefix}/name`);

  // Reactive subscription: ctx.documents (OptimisticClient) is a plain-callback store, not a
  // Svelte rune — every $derived.by that reads it directly must call subscribe() itself
  // (mirrors GameSettingsPanel's ws/lgDoc/vmDoc/scenes pattern), or the derived value freezes
  // at first read and never observes later edits, corrupting the OCC `old` on any second edit.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const name = $derived.by((): string | null => (doc ? (getPointer(doc, namePrefix) as string | null | undefined) ?? null : null));
  const engine = $derived.by((): ActorEngine | undefined => (doc ? (getPointer(doc, enginePrefix) as ActorEngine | undefined) : undefined));
  // The genuinely game-system-owned `system` body (untouched by the three-band re-root),
  // still the SystemTreeEditor's root — distinct from the engine band above.
  const sysBody = $derived.by((): unknown => (doc ? getPointer(doc, systemPrefix) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  const factions = $derived.by((): [string, {
    /** The faction's display label, shown in the `<select>` option below. */
    name: string;
  }][] => {
    subscribe();
    const reg = ctx.documents.query("faction-registry")[0]?.engine as FactionRegistryEngine | undefined;
    return Object.entries(reg?.factions ?? {});
  });

  // Inventory: only embedded items directly under an actor doc (systemPrefix "/system")
  // are one-level openable via openDocument; a deeply-nested (instanced-token) actor fails
  // safe and shows no inventory section. `/embedded/item/<i>` is the openDocument embedded ref.
  const inventory = $derived.by((): {
    /** The embedded item's display label, or a fallback i18n string when unnamed. */
    name: string;
    /** The `openDocument` embedded ref (`/embedded/item/<i>`) — see the comment above. */
    path: string;
  }[] => {
    subscribe();
    if (!doc || systemPrefix !== "/system") return [];
    return (doc.embedded?.item ?? []).map((it, i) => ({
      name: it.name ?? t("sheetActor.unnamedItem"),
      path: `/embedded/item/${i}`,
    }));
  });

  /** Update an engine-owned field (`/engine/<field>`).
   * @param field The field name under `/engine` to write (e.g. `"displayName"`).
   * @param value The new value for that field.
   * @example
   * ```
   * // private function; not part of the public API — wired to each field
   * // control's onchange below
   * setEngine("displayName", "Ancient Red Dragon");
   * ```
   */
  function setEngine(field: string, value: unknown): void {
    if (!doc) return;
    const path = `${enginePrefix}/${field}`;
    setField(ctx, docId, path, getPointer(doc, path), value);
  }

  /** Update the envelope `name` field.
   * @param value The new name.
   * @example
   * ```
   * // private function; not part of the public API — wired to the name
   * // input's onchange above
   * setName("Ancient Red Dragon");
   * ```
   */
  function setName(value: string): void {
    if (!doc) return;
    setField(ctx, docId, namePrefix, name, value);
  }
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{engine ? actorDisplayName({ name, displayName: engine.displayName }) : t("sheets.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>

  {#if doc && engine}
    <div class="fields">
      <label>{t("sheetActor.name")}
        <input aria-label={t("sheetActor.name")} value={name ?? ""} disabled={readOnly}
          onchange={(e) => setName((e.currentTarget as HTMLInputElement).value)} /></label>
      <label>{t("sheetActor.displayName")}
        <input aria-label={t("sheetActor.displayName")} value={engine.displayName ?? ""} disabled={readOnly}
          onchange={(e) => setEngine("displayName", (e.currentTarget as HTMLInputElement).value)} /></label>
      <label>{t("sheetActor.faction")}
        <select aria-label={t("sheetActor.faction")} value={engine.faction ?? ""} disabled={readOnly}
          onchange={(e) => setEngine("faction", (e.currentTarget as HTMLSelectElement).value || null)}>
          <option value="">{t("sheetActor.noFaction")}</option>
          {#each factions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
        </select></label>
      <label>{t("sheetActor.shape")}
        <select aria-label={t("sheetActor.shape")} value={engine.shape} disabled={readOnly}
          onchange={(e) => setEngine("shape", (e.currentTarget as HTMLSelectElement).value)}>
          <option value="square">{t("actors.shapeSquare")}</option><option value="circle">{t("actors.shapeCircle")}</option>
        </select></label>
      <label>{t("sheetActor.sizeW")}
        <input type="number" min="0" step="0.5" aria-label={t("sheetActor.sizeW")} value={engine.size?.w ?? 1} disabled={readOnly}
          onchange={(e) => {
            const w = Number((e.currentTarget as HTMLInputElement).value);
            if (Number.isNaN(w)) return;
            setEngine("size", { w, h: engine.size?.h ?? 1 });
          }} /></label>
      <label>{t("sheetActor.sizeH")}
        <input type="number" min="0" step="0.5" aria-label={t("sheetActor.sizeH")} value={engine.size?.h ?? 1} disabled={readOnly}
          onchange={(e) => {
            const h = Number((e.currentTarget as HTMLInputElement).value);
            if (Number.isNaN(h)) return;
            setEngine("size", { w: engine.size?.w ?? 1, h });
          }} /></label>
    </div>

    {#if inventory.length > 0}
      <h3>{t("sheetActor.inventory")}</h3>
      <ul class="inventory">
        {#each inventory as item (item.path)}
          <li><button type="button" onclick={() => ctx.openDocument({ docId, embeddedPath: item.path })}>{item.name}</button></li>
        {/each}
      </ul>
    {/if}

    <details>
      <summary>{t("sheetActor.system")}</summary>
      <SystemTreeEditor {doc} basePath={systemPrefix} root={sysBody} {readOnly} />
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
  .fields { display: flex; flex-direction: column; gap: var(--space-1); }
  label { display: flex; flex-direction: column; gap: 2px; }
  .inventory { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .inventory button { min-height: 44px; text-align: left; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .inventory button:focus-visible { outline: 2px solid var(--accent); }
  .missing { opacity: 0.7; font-style: italic; }
</style>
