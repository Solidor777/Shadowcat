<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { bulkPatchAssets, deleteAsset, type WireDocument } from "@shadowcat/core";
  import { ASSET_FOLDER_DOC_TYPE } from "./folderOps";

  let {
    selected,
    onChanged,
  }: {
    /** The selected asset ids the bar operates on. */
    selected: string[];
    /** Called after any successful bulk mutation so the browser refetches. */
    onChanged: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const folders = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query(ASSET_FOLDER_DOC_TYPE);
  });

  let movePickerOpen = $state(false);
  let tagDraft = $state("");
  let confirmingDelete = $state(false);
  let busy = $state(false);

  /** Runs one bulk mutation with the shared busy/error/refresh handling.
   * @param op - The REST call(s) to run.
   * @example
   * ```
   * // private function; wraps every handler below
   * void run(async () => {});
   * ```
   */
  async function run(op: () => Promise<unknown>): Promise<void> {
    busy = true;
    try {
      await op();
      onChanged();
    } catch (e) {
      ctx.notify(t("assetBrowser.error", { message: String(e) }));
    } finally {
      busy = false;
    }
  }

  /** Sends one tag delta (add or remove) for every selected id.
   * @param direction - Which side of the delta the drafted tag goes to.
   * @example
   * ```
   * // private function; wired to the add/remove buttons below
   * tagDelta("add");
   * ```
   */
  function tagDelta(direction: "add" | "remove"): void {
    const tag = tagDraft.trim();
    tagDraft = "";
    if (!tag) return;
    void run(() =>
      bulkPatchAssets(ctx.world, {
        ids: selected,
        add_tags: direction === "add" ? [tag] : [],
        remove_tags: direction === "remove" ? [tag] : [],
      }),
    );
  }
</script>

<div class="bulk-bar" data-testid="bulk-bar">
  <span class="count">{t("assetBrowser.bulkCount", { count: String(selected.length) })}</span>

  <button
    type="button"
    data-testid="bulk-move"
    disabled={busy}
    onclick={() => (movePickerOpen = !movePickerOpen)}
  >{t("assetBrowser.bulkMove")}</button>
  {#if movePickerOpen}
    <span class="picker" role="menu">
      {#each folders as f (f.id)}
        <button
          type="button"
          data-testid={"bulk-move-target-" + f.id}
          onclick={() => {
            movePickerOpen = false;
            void run(() =>
              bulkPatchAssets(ctx.world, {
                ids: selected,
                folder_id: f.id,
                add_tags: [],
                remove_tags: [],
              }),
            );
          }}
        >{f.name}</button>
      {/each}
    </span>
  {/if}

  <input
    data-testid="bulk-tag-input"
    type="text"
    placeholder={t("assetBrowser.bulkTag")}
    bind:value={tagDraft}
  />
  <button type="button" data-testid="bulk-tag-add" disabled={busy} onclick={() => tagDelta("add")}>
    {t("assetBrowser.bulkTagAdd")}
  </button>
  <button
    type="button"
    data-testid="bulk-tag-remove"
    disabled={busy}
    onclick={() => tagDelta("remove")}
  >{t("assetBrowser.bulkTagRemove")}</button>

  {#if confirmingDelete}
    <button
      type="button"
      class="danger"
      data-testid="bulk-delete-confirm"
      disabled={busy}
      onclick={() => {
        confirmingDelete = false;
        void run(async () => {
          for (const id of selected) await deleteAsset(id);
        });
      }}
    >{t("assetBrowser.deleteConfirm")}</button>
    <button type="button" onclick={() => (confirmingDelete = false)}>
      {t("assetBrowser.pickCancel")}
    </button>
  {:else}
    <button
      type="button"
      class="danger"
      data-testid="bulk-delete"
      disabled={busy}
      onclick={() => (confirmingDelete = true)}
    >{t("assetBrowser.delete")}</button>
  {/if}
</div>

<style lang="scss">
  .bulk-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem;
    border-top: 1px solid var(--border);
    background: var(--surface-raised);
    button,
    input {
      min-height: 2rem;
    }
  }
  .count {
    font-size: 0.85rem;
    color: var(--text-muted);
  }
  .picker {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    button {
      border: 1px solid var(--border);
    }
  }
  .danger {
    color: var(--danger, #c33);
  }
</style>
