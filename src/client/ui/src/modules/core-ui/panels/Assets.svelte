<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "../../../lib/appContext";
  import { listAssets, uploadAsset, replaceAsset, deleteAsset } from "../../../lib/api";

  const { world, assets: resolver, onAssetChanged, t } = getAppContext();

  let items = $state<Asset[]>([]);
  let selectedId = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function reload(): Promise<void> {
    try {
      items = await listAssets(world);
      error = null;
    } catch (e) {
      error = t("assets.error", { message: String(e) });
    }
  }

  // Load on mount; reload whenever another client (or our own replace/delete)
  // broadcasts an AssetChanged. The resolver was already cache-busted by
  // WorldSession before this fires, so re-rendered <img> tags pull fresh bytes.
  $effect(() => {
    void reload();
    return onAssetChanged(() => void reload());
  });

  async function onUpload(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    try {
      await uploadAsset(world, file);
      await reload();
    } catch (err) {
      error = t("assets.error", { message: String(err) });
    }
  }

  async function onReplace(uuid: string, e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    try {
      await replaceAsset(uuid, file);
      // The asset_changed{replaced} broadcast drives the reload + cache-bust.
    } catch (err) {
      error = t("assets.error", { message: String(err) });
    }
  }

  async function onDelete(uuid: string): Promise<void> {
    try {
      await deleteAsset(uuid);
      if (selectedId === uuid) selectedId = null;
    } catch (err) {
      error = t("assets.error", { message: String(err) });
    }
  }
</script>

<section class="assets">
  <h2>{t("assets.title")}</h2>

  <label class="upload">
    <span>{t("assets.upload")}</span>
    <input type="file" accept="image/*" onchange={onUpload} data-testid="asset-upload" />
  </label>

  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if items.length === 0}
    <p class="empty">{t("assets.empty")}</p>
  {:else}
    <ul class="grid">
      {#each items as a (a.id)}
        <li class="tile" class:selected={selectedId === a.id} data-testid="asset-tile">
          <button class="thumb" type="button" onclick={() => (selectedId = a.id)}>
            <img src={resolver.url(a.id)} alt={a.original_name} />
          </button>
          <span class="name">{a.original_name}</span>
          <div class="row">
            <label class="replace">
              <span>{t("assets.replace")}</span>
              <input type="file" accept="image/*" onchange={(e) => onReplace(a.id, e)} />
            </label>
            <button type="button" onclick={() => onDelete(a.id)}>{t("assets.delete")}</button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  {#if selectedId}
    <p class="selected" data-testid="selected-id">{t("assets.selected", { id: selectedId })}</p>
  {/if}
</section>

<style lang="scss">
  .assets {
    padding: var(--space-4);
    display: grid;
    gap: var(--space-3);
  }
  .grid {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(7rem, 1fr));
    gap: var(--space-3);
  }
  .tile {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
  }
  .tile.selected {
    border-color: var(--accent);
  }
  .thumb {
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
  }
  .thumb img {
    width: 100%;
    aspect-ratio: 1;
    object-fit: cover;
    border-radius: var(--radius-1);
    display: block;
  }
  .name {
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }
  .error {
    color: var(--danger);
  }
</style>
