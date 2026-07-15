<script lang="ts">
  import { getAppContext, SystemTreeEditor, setField } from "@shadowcat/ui-kit";
  import { getPointer, actorDisplayName, type WireDocument, type ActorSystem, type FactionRegistrySystem } from "@shadowcat/core";

  // Actor sheet: engine-known fields (name, display name, faction, shape, size) as real
  // controls + the opaque `system` body as a tree editor + the embedded-items inventory
  // (opens each via openDocument). Reads the OPTIMISTIC store (per-recipient redaction +
  // OwnerOrGm naming are free). Every edit's `old` is the RAW current stored value.
  let { docId, systemPrefix, close }: { docId: string; systemPrefix: string; close: () => void } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const doc = $derived.by((): WireDocument | undefined => ctx.documents.get(docId));
  const system = $derived.by((): ActorSystem | undefined => (doc ? (getPointer(doc, systemPrefix) as ActorSystem | undefined) : undefined));
  const readOnly = $derived(!doc || !ctx.canEdit(doc, systemPrefix));

  const factions = $derived.by((): [string, { name: string }][] => {
    const reg = ctx.documents.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
    return Object.entries(reg?.factions ?? {});
  });

  // Inventory: only embedded items directly under an actor doc (systemPrefix "/system")
  // are one-level openable via openDocument; a deeply-nested (instanced-token) actor lists
  // them read-only. `/embedded/item/<i>` is the openDocument embedded ref.
  const inventory = $derived.by((): { name: string; path: string }[] => {
    if (!doc || systemPrefix !== "/system") return [];
    return (doc.embedded?.item ?? []).map((it, i) => ({
      name: (it.system as { name?: string } | undefined)?.name ?? t("sheetActor.unnamedItem"),
      path: `/embedded/item/${i}`,
    }));
  });

  function set(field: string, value: unknown): void {
    if (!doc) return;
    const path = `${systemPrefix}/${field}`;
    setField(ctx, docId, path, getPointer(doc, path), value);
  }
</script>

<div class="sheet" role="dialog" aria-label={t("sheets.title")}>
  <header class="sheet-header">
    <h2>{system ? actorDisplayName(system) : t("sheets.title")}</h2>
    <button type="button" class="close" aria-label={t("sheets.close")} onclick={close}>×</button>
  </header>

  {#if doc && system}
    <div class="fields">
      <label>{t("sheetActor.name")}
        <input aria-label="sheetActor.name" value={system.name ?? ""} disabled={readOnly}
          onchange={(e) => set("name", (e.currentTarget as HTMLInputElement).value)} /></label>
      <label>{t("sheetActor.displayName")}
        <input aria-label="sheetActor.displayName" value={system.displayName ?? ""} disabled={readOnly}
          onchange={(e) => set("displayName", (e.currentTarget as HTMLInputElement).value)} /></label>
      <label>{t("sheetActor.faction")}
        <select aria-label="sheetActor.faction" value={system.faction ?? ""} disabled={readOnly}
          onchange={(e) => set("faction", (e.currentTarget as HTMLSelectElement).value || null)}>
          <option value="">{t("sheetActor.noFaction")}</option>
          {#each factions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
        </select></label>
      <label>{t("sheetActor.shape")}
        <select aria-label="sheetActor.shape" value={system.shape} disabled={readOnly}
          onchange={(e) => set("shape", (e.currentTarget as HTMLSelectElement).value)}>
          <option value="square">square</option><option value="circle">circle</option>
        </select></label>
      <label>{t("sheetActor.sizeW")}
        <input type="number" min="0" step="0.5" aria-label="sheetActor.sizeW" value={system.size?.w ?? 1} disabled={readOnly}
          onchange={(e) => set("size", { w: Number((e.currentTarget as HTMLInputElement).value), h: system.size?.h ?? 1 })} /></label>
      <label>{t("sheetActor.sizeH")}
        <input type="number" min="0" step="0.5" aria-label="sheetActor.sizeH" value={system.size?.h ?? 1} disabled={readOnly}
          onchange={(e) => set("size", { w: system.size?.w ?? 1, h: Number((e.currentTarget as HTMLInputElement).value) })} /></label>
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
  .fields { display: flex; flex-direction: column; gap: var(--space-1); }
  label { display: flex; flex-direction: column; gap: 2px; }
  .inventory { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .inventory button { min-height: 44px; text-align: left; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  .inventory button:focus-visible { outline: 2px solid var(--accent); }
  .missing { opacity: 0.7; font-style: italic; }
</style>
