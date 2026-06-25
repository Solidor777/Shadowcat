<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildActorDoc, setNameHidden, actorDisplayName, listAssets, type ActorSystem, type WireDocument, type FactionRegistrySystem, type Faction } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  // Reactive read of the document store (same bridge as Surface.svelte): reading
  // `subscribe()` inside the derived registers a dependency so the list re-renders on create.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const actorDocs = $derived.by(() => {
    subscribe();
    return ctx.documents.query("actor");
  });

  let name = $state("");
  let displayName = $state("");
  let assetId = $state<string | null>(null);
  let instanceOnDrop = $state(true);
  let hideName = $state(false);
  let faction = $state<string | null>(null);
  let shape = $state<"square" | "circle">("square");
  let sizeW = $state(1);
  let sizeH = $state(1);
  let darkvision = $state(0);
  let assetList = $state<Asset[]>([]);

  const factionOptions = $derived.by((): [string, Faction][] => {
    subscribe();
    const reg = ctx.documents.query("faction-registry")[0]?.system as FactionRegistrySystem | undefined;
    return Object.entries(reg?.factions ?? {});
  });

  const isHidden = (a: WireDocument): boolean => a.permissions.property_overrides["/system/name"] === "owner_or_gm";

  function toggleHidden(a: WireDocument): void {
    const cur = a.permissions.property_overrides;
    const next = { ...cur };
    if (next["/system/name"] === "owner_or_gm") delete next["/system/name"];
    else next["/system/name"] = "owner_or_gm";
    ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/permissions/property_overrides", old: cur, new: next }] }]);
  }

  function refreshAssets(): void {
    void listAssets(ctx.world).then((a) => (assetList = a.filter((x) => x.content_type.startsWith("image/"))));
  }
  $effect(() => {
    refreshAssets();
    return ctx.onAssetChanged(refreshAssets);
  });

  function create(): void {
    if (!name || !assetId) return;
    const system: ActorSystem = {
      name,
      displayName: displayName || name,
      visual: { kind: "image", asset: assetId },
      size: { w: sizeW, h: sizeH },
      shape,
      faction,
      conditions: [],
      prototype: instanceOnDrop,
      ...(darkvision > 0 ? { vision: [{ mode: "darkvision" as const, range: darkvision }] } : {}),
    };
    const doc = buildActorDoc(ctx.world, system);
    if (hideName) setNameHidden(doc, true);
    ctx.dispatchIntent([{ op: "create", doc }]);
    name = "";
    displayName = "";
    assetId = null;
    hideName = false;
    faction = null;
    shape = "square";
    sizeW = 1;
    sizeH = 1;
    darkvision = 0;
  }
</script>

<section class="actors">
  <h3>{t("actors.title")}</h3>
  <ul class="list">
    {#each actorDocs as a (a.id)}
      <li>
        <button
          type="button"
          class:selected={ctx.actorSelection.selectedId === a.id}
          onclick={() => ctx.actorSelection.select(a.id)}
        >{actorDisplayName(a.system as { name?: string; displayName?: string })}</button>
        {#if ctx.role === "gm"}
          <button type="button" class="hide-toggle" onclick={() => toggleHidden(a)}>
            {isHidden(a) ? t("actors.nameShown") : t("actors.hideName")}
          </button>
          <select
            aria-label={t("actors.faction")}
            value={(a.system as { faction?: string | null }).faction ?? ""}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/faction", old: (a.system as { faction?: string | null }).faction ?? null, new: e.currentTarget.value || null }] }])}
          >
            <option value="">—</option>
            {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
          </select>
          <select
            aria-label={t("actors.shape")}
            value={(a.system as { shape?: string }).shape ?? "square"}
            onchange={(e) => ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/shape", old: (a.system as { shape?: string }).shape ?? "square", new: e.currentTarget.value }] }])}
          >
            <option value="square">{t("actors.shapeSquare")}</option>
            <option value="circle">{t("actors.shapeCircle")}</option>
          </select>
          <!-- Per-row size inputs dispatch an update op (not bind:value), so e.currentTarget.value
               is a string; Number(...) coerces it to keep system.size numeric for actor.size × cell math. -->
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.width")}
            value={(a.system as { size?: { w: number } }).size?.w ?? 1}
            onchange={(e) => { const sz = (a.system as { size?: { w: number; h: number } }).size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/size", old: sz, new: { w: Number(e.currentTarget.value), h: sz.h } }] }]); }}
          />
          <input
            type="number" min="0.5" step="0.5" class="size-edit" aria-label={t("actors.height")}
            value={(a.system as { size?: { h: number } }).size?.h ?? 1}
            onchange={(e) => { const sz = (a.system as { size?: { w: number; h: number } }).size ?? { w: 1, h: 1 }; ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/size", old: sz, new: { w: sz.w, h: Number(e.currentTarget.value) } }] }]); }}
          />
          <!-- Per-row darkvision input dispatches an update to /system/vision; range=0 clears to empty array. -->
          <input
            type="number" min="0" step="1" class="size-edit" aria-label={t("actors.darkvision")}
            value={(a.system as { vision?: Array<{ mode: string; range: number }> }).vision?.find((v) => v.mode === "darkvision")?.range ?? 0}
            onchange={(e) => { const range = Number(e.currentTarget.value); ctx.dispatchIntent([{ op: "update", doc_id: a.id, changes: [{ path: "/system/vision", old: null, new: range > 0 ? [{ mode: "darkvision", range }] : [] }] }]); }}
          />
        {/if}
      </li>
    {/each}
  </ul>
  <label class="keep">
    <input
      type="checkbox"
      checked={ctx.actorSelection.keepAfterPlace}
      onchange={(e) => ctx.actorSelection.setKeepAfterPlace(e.currentTarget.checked)}
    />
    {t("actors.keepAfterPlace")}
  </label>
  <form onsubmit={(e) => { e.preventDefault(); create(); }}>
    <input placeholder={t("actors.name")} aria-label={t("actors.name")} bind:value={name} />
    <input placeholder={t("actors.displayName")} aria-label={t("actors.displayName")} bind:value={displayName} />
    <label><input type="checkbox" bind:checked={instanceOnDrop} /> {t("actors.instanceOnDrop")}</label>
    <label><input type="checkbox" bind:checked={hideName} /> {t("actors.hideName")}</label>
    <label>{t("actors.faction")}
      <select bind:value={faction}>
        <option value={null}>—</option>
        {#each factionOptions as [id, f] (id)}<option value={id}>{f.name}</option>{/each}
      </select>
    </label>
    <label>{t("actors.shape")}
      <select bind:value={shape}>
        <option value="square">{t("actors.shapeSquare")}</option>
        <option value="circle">{t("actors.shapeCircle")}</option>
      </select>
    </label>
    <label>{t("actors.size")}
      <input type="number" min="0.5" step="0.5" aria-label={t("actors.width")} bind:value={sizeW} />
      <input type="number" min="0.5" step="0.5" aria-label={t("actors.height")} bind:value={sizeH} />
    </label>
    <label>
      {t("actors.darkvision")}
      <!-- Uses value+onchange (not bind:value) so fireEvent.change updates state in tests. -->
      <input type="number" min="0" step="1" aria-label="actors.darkvision" value={darkvision} onchange={(e) => (darkvision = Number(e.currentTarget.value))} oninput={(e) => (darkvision = Number(e.currentTarget.value))} />
    </label>
    <div class="picker">
      {#each assetList as a (a.id)}
        <button type="button" class:selected={assetId === a.id} title={a.original_name} onclick={() => (assetId = a.id)}>
          <img src={ctx.assets.url(a.id)} alt={a.original_name} />
        </button>
      {/each}
    </div>
    <button type="submit" disabled={!name || !assetId}>{t("actors.create")}</button>
  </form>
</section>

<style lang="scss">
  .actors {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list button {
    min-height: 44px;
    width: 100%;
    text-align: left;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .list button.selected {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .picker {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .picker button {
    padding: 0;
    border: 2px solid transparent;
    border-radius: var(--radius-1);
    background: none;
    cursor: pointer;
  }
  .picker button.selected {
    border-color: var(--accent);
  }
  .picker img {
    width: 48px;
    height: 48px;
    object-fit: cover;
    display: block;
  }
  input,
  label,
  button[type="submit"] {
    min-height: 32px;
  }
</style>
