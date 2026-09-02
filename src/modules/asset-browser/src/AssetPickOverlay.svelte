<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import AssetBrowser from "./AssetBrowser.svelte";

  // Renders the pick-mode modal whenever a `pickAsset` request is pending.
  // Contributed into `shadowcat.surface:overlay` (outside the layout grid);
  // usable by ANY member — the browser PANEL stays GM-only, this does not.
  const ctx = getAppContext();

  const pending = $derived(ctx.assetPick.pending);

  let dialogEl = $state<HTMLDivElement | null>(null);

  // Scrim/Escape/focus pattern shared with the merge-conflict modal: focus
  // the dialog on open; window-level Escape cancels.
  $effect(() => {
    if (pending) dialogEl?.focus();
  });
  $effect(() => {
    if (!pending) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") ctx.assetPick.settle(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
</script>

{#if pending}
  <div class="modal-scrim" data-testid="asset-pick-scrim">
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label={ctx.t("assetBrowser.pickTitle")}
      tabindex="-1"
      data-testid="asset-pick-dialog"
      bind:this={dialogEl}
    >
      {#key pending}
        <AssetBrowser
          mode="pick"
          initialFilters={pending.opts}
          onConfirm={(ids) => ctx.assetPick.settle(ids)}
          onCancel={() => ctx.assetPick.settle(null)}
        />
      {/key}
    </div>
  </div>
{/if}

<style lang="scss">
  .modal-scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }
  .modal {
    width: min(56rem, 94vw);
    height: min(36rem, 88vh);
    background: var(--surface-base);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    > :global(.asset-browser) {
      flex: 1;
      min-height: 0;
    }
  }
</style>
