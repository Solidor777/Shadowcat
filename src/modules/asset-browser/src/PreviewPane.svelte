<script lang="ts">
  import type { Asset } from "@shadowcat/types";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { patchAsset, deleteAsset, reconvertAsset, originalUrl } from "@shadowcat/core";

  let {
    asset,
    mutable,
    onChanged,
  }: {
    /** The selected asset. */
    asset: Asset;
    /** Whether mutation affordances render (pick mode passes `false`). */
    mutable: boolean;
    /** Called after any successful mutation so the browser refetches. */
    onChanged: () => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let tagDraft = $state("");
  let renaming = $state(false);
  let renameDraft = $state("");
  let confirmingDelete = $state(false);
  let busy = $state(false);

  /** Runs one mutation with the shared busy/error/refresh handling.
   * @param op - The REST call to run.
   * @example
   * ```
   * // private function; wraps every mutation handler below
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

  /** Commits the tag input: patches the FULL replacement explicit set.
   * @example
   * ```
   * // private function; wired to the tag input's Enter handler below
   * commitTag();
   * ```
   */
  function commitTag(): void {
    const tag = tagDraft.trim();
    tagDraft = "";
    if (!tag || asset.tags.includes(tag)) return;
    void run(() => patchAsset(asset.id, { tags: [...asset.tags, tag] }));
  }

  /** A human-readable byte size.
   * @param n - Byte count.
   * @returns The formatted size.
   * @example
   * ```
   * // private helper; used by the metadata rows below
   * fmtBytes(2048); // "2.0 KiB"
   * ```
   */
  function fmtBytes(n: number | bigint): string {
    const v = Number(n);
    if (v < 1024) return `${v} B`;
    if (v < 1024 * 1024) return `${(v / 1024).toFixed(1)} KiB`;
    return `${(v / (1024 * 1024)).toFixed(1)} MiB`;
  }
</script>

<div class="preview-pane" data-testid="preview-pane">
  <img class="preview" src={ctx.assets.url(asset.id, "preview")} alt={asset.original_name} />

  {#if renaming && mutable}
    <input
      data-testid="preview-rename-input"
      type="text"
      bind:value={renameDraft}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          renaming = false;
          const next = renameDraft.trim();
          if (next && next !== asset.original_name)
            void run(() => patchAsset(asset.id, { name: next }));
        } else if (e.key === "Escape") {
          renaming = false;
        }
      }}
    />
  {:else}
    <h3 class="name">
      {asset.original_name}
      {#if mutable}
        <button
          type="button"
          class="mini"
          data-testid="preview-rename"
          title={t("assetBrowser.rename")}
          onclick={() => {
            renaming = true;
            renameDraft = asset.original_name;
          }}
        >✎</button>
      {/if}
    </h3>
  {/if}

  <dl class="meta">
    {#if asset.width != null && asset.height != null}
      <dt>{t("assetBrowser.metaDimensions")}</dt>
      <dd>{asset.width}×{asset.height}</dd>
    {/if}
    <dt>{t("assetBrowser.metaType")}</dt>
    <dd>{asset.content_type}</dd>
    <dt>{t("assetBrowser.metaSize")}</dt>
    <dd>{fmtBytes(asset.byte_size)}</dd>
    {#if asset.original_retained}
      <dt>{t("assetBrowser.metaOriginal")}</dt>
      <dd>{asset.original_content_type} · {fmtBytes(asset.original_byte_size)}</dd>
    {/if}
    {#if asset.conversion_note}
      <dt>{t("assetBrowser.metaNote")}</dt>
      <dd>{asset.conversion_note}</dd>
    {/if}
  </dl>

  <div class="tags">
    {#each asset.tags as tag (tag)}
      <span class="chip">
        {tag}
        {#if mutable}
          <button
            type="button"
            data-testid={"preview-tag-remove-" + tag}
            aria-label={t("assetBrowser.removeTag", { tag })}
            onclick={() =>
              void run(() =>
                patchAsset(asset.id, { tags: asset.tags.filter((x) => x !== tag) }),
              )}
          >×</button>
        {/if}
      </span>
    {/each}
    {#each asset.derived_tags as tag (tag)}
      <span class="chip derived" data-testid={"preview-derived-" + tag}>{tag}</span>
    {/each}
    {#if mutable}
      <input
        data-testid="preview-tag-input"
        type="text"
        placeholder={t("assetBrowser.addTag")}
        bind:value={tagDraft}
        onkeydown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commitTag();
          }
        }}
      />
    {/if}
  </div>

  {#if mutable}
    <div class="actions">
      {#if asset.original_retained}
        <a data-testid="preview-download-original" href={originalUrl(asset.id)} download>
          {t("assetBrowser.downloadOriginal")}
        </a>
      {/if}
      <button
        type="button"
        data-testid="preview-reconvert"
        disabled={!asset.original_retained || busy}
        onclick={() => void run(() => reconvertAsset(asset.id))}
      >{t("assetBrowser.reconvert")}</button>
      {#if confirmingDelete}
        <button
          type="button"
          class="danger"
          data-testid="preview-delete-confirm"
          disabled={busy}
          onclick={() => {
            confirmingDelete = false;
            void run(() => deleteAsset(asset.id));
          }}
        >{t("assetBrowser.deleteConfirm")}</button>
        <button type="button" onclick={() => (confirmingDelete = false)}>
          {t("assetBrowser.pickCancel")}
        </button>
      {:else}
        <button
          type="button"
          class="danger"
          data-testid="preview-delete"
          disabled={busy}
          onclick={() => (confirmingDelete = true)}
        >{t("assetBrowser.delete")}</button>
      {/if}
    </div>
  {/if}
</div>

<style lang="scss">
  .preview-pane {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.5rem;
  }
  .preview {
    max-width: 100%;
    max-height: 12rem;
    object-fit: contain;
    background: var(--surface-raised);
  }
  .name {
    margin: 0;
    font-size: 0.95rem;
    word-break: break-all;
  }
  .meta {
    margin: 0;
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.125rem 0.5rem;
    font-size: 0.8rem;
    dt {
      color: var(--text-muted);
    }
    dd {
      margin: 0;
    }
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.125rem;
    padding: 0.125rem 0.375rem;
    border: 1px solid var(--border);
    border-radius: 1rem;
    font-size: 0.8rem;
    &.derived {
      opacity: 0.65;
      border-style: dashed;
    }
    button {
      border: none;
      background: none;
      cursor: pointer;
      color: var(--text-muted);
    }
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    button,
    a {
      min-height: 2rem;
    }
  }
  .mini {
    border: none;
    background: none;
    cursor: pointer;
  }
  .danger {
    color: var(--danger, #c33);
  }
  input {
    min-height: 2rem;
  }
</style>
