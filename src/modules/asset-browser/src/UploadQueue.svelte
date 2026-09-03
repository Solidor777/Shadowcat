<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { UploadQueue } from "./uploadQueueModel.svelte";

  let {
    queue,
  }: {
    /** The browser's upload queue (stable ref). */
    queue: UploadQueue;
  } = $props();

  const { t } = getAppContext();

  const visible = $derived(queue.entries.length > 0);
</script>

{#if visible}
  <div class="upload-queue" data-testid="upload-queue">
    {#each queue.entries as e, i (e)}
      <div class="entry" data-testid="upload-entry" data-status={e.status}>
        <span class="name">{e.file.name}</span>
        {#if e.status === "uploading" || e.status === "queued"}
          <progress max={e.total} value={e.sent}></progress>
          <button type="button" data-testid={"upload-cancel-" + i} onclick={() => queue.cancel(i)}>
            {t("assetBrowser.pickCancel")}
          </button>
        {:else if e.status === "error"}
          <span class="error">{e.error}</span>
          <button type="button" data-testid={"upload-retry-" + i} onclick={() => queue.retry(i)}>
            {t("assetBrowser.uploadRetry")}
          </button>
        {:else if e.error}
          <span class="warn">{t("assetBrowser.uploadPartial")}</span>
        {:else}
          <span class="ok">✓</span>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style lang="scss">
  .upload-queue {
    flex: none;
    max-height: 8rem;
    overflow-y: auto;
    border-top: 1px solid var(--border);
    padding: 0.25rem 0.375rem;
    font-size: 0.8rem;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    min-height: 1.75rem;
    .name {
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    progress {
      width: 6rem;
    }
    .error {
      color: var(--danger);
    }
    .warn {
      color: var(--warning);
    }
    .ok {
      color: var(--success);
    }
  }
</style>
