<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { buildActorDoc, listAssets, type ActorSystem } from "@shadowcat/core";

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
  let assetList = $state<Asset[]>([]);

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
      size: { w: 1, h: 1 },
      shape: "square",
      faction: null,
      conditions: [],
      prototype: instanceOnDrop,
    };
    ctx.dispatchIntent([{ op: "create", doc: buildActorDoc(ctx.world, system) }]);
    name = "";
    displayName = "";
    assetId = null;
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
        >{(a.system as { name?: string }).name ?? a.id}</button>
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
