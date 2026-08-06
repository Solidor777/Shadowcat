<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { listAssets, uploadAsset, replaceAsset, deleteAsset } from "@shadowcat/core";

  const { world, assets: resolver, onAssetChanged, t } = getAppContext();

  let items = $state<Asset[]>([]);
  let selectedId = $state<string | null>(null);
  let error = $state<string | null>(null);

  /** Repopulates `items` from the server — the source of truth for the asset
   * grid (assets are plain REST resources, not `ctx.documents` entries, so no
   * store subscription applies here). Called on mount, whenever an
   * `AssetChanged` broadcast lands (see the effect below), and by
   * `onUpload`/`onDelete` for an immediate refresh that doesn't wait on a
   * broadcast round-trip.
   * @returns Resolves once `items` holds the fresh list, or `error` is set on
   * a fetch failure.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the mount
   * // effect below and from the mutation handlers that need an immediate refresh
   * void reload();
   * ```
   */
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
  // WorldSession before this fires (its `enter`'s `onAssetChanged` handler bumps
  // `this.assets` BEFORE notifying listeners like this effect), so re-rendered
  // <img> tags pull fresh bytes.
  $effect(() => {
    void reload();
    return onAssetChanged(() => void reload());
  });

  /** Uploads the selected file as a brand-new asset, then explicitly reloads
   * the grid. `assets::upload` never broadcasts `AssetChanged` (only `assets::replace`/
   * `assets::delete`
   * do), so a freshly-created asset has no broadcast round-trip to
   * react to; this component must refresh itself.
   * @param e The `<input type="file">` change event; the input's value is
   * reset so choosing the same filename again still fires `onchange`.
   * @example
   * ```
   * // private function; not part of the public API — wired to the upload
   * // input's onchange below
   * void onUpload(event);
   * ```
   */
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

  /** Replaces the asset's bytes behind its stable `uuid`, without an explicit
   * reload. Refresh here is driven entirely by the server's out-of-band
   * `asset_changed{replaced}` broadcast (`Room::broadcast_aux` — best-effort, dropped
   * if there are no
   * receivers, and never replayed on resync): the `onAssetChanged` effect
   * above both reloads `items` and lets `resolver` bump its cache-busting
   * revision so the `<img>` tag re-requests fresh bytes. If that one broadcast
   * is lost — e.g. a receiver briefly disconnected when it fires — nothing
   * else in this component notices: `AssetResolver.url` only cache-busts in
   * response to `onAssetChanged`, not
   * from the asset's server-side `version`, so the tile keeps its pre-replace
   * URL and may go on being served from the browser cache until some
   * unrelated reload happens.
   * @param uuid The asset's stable id (unchanged by a replace).
   * @param e The `<input type="file">` change event; the input's value is
   * reset so choosing the same filename again still fires `onchange`.
   * @example
   * ```
   * // private function; not part of the public API — wired to each tile's
   * // replace input's onchange below
   * void onReplace(uuid, event);
   * ```
   */
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

  /** Deletes the asset and immediately reloads the grid (see the inline
   * comment below for why the explicit reload doesn't wait on the broadcast).
   * @param uuid The asset's stable id.
   * @example
   * ```
   * // private function; not part of the public API — wired to each tile's
   * // delete button's onclick below
   * void onDelete(uuid);
   * ```
   */
  async function onDelete(uuid: string): Promise<void> {
    try {
      await deleteAsset(uuid);
      if (selectedId === uuid) selectedId = null;
      // Flush immediately rather than waiting on the AssetChanged{deleted}
      // broadcast round-trip (which also reloads, idempotently) — no stale tile.
      await reload();
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
