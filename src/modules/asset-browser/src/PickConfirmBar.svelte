<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";

  let {
    count,
    onConfirm,
    onCancel,
  }: {
    /** How many assets are currently selected. */
    count: number;
    /** Confirms the current selection. */
    onConfirm: () => void;
    /** Dismisses the pick without a choice. */
    onCancel: () => void;
  } = $props();

  const { t } = getAppContext();
</script>

<footer class="pick-bar" data-testid="pick-bar">
  <span class="count">{t("assetBrowser.bulkCount", { count: String(count) })}</span>
  <button type="button" data-testid="pick-cancel" onclick={onCancel}>
    {t("assetBrowser.pickCancel")}
  </button>
  <button type="button" data-testid="pick-confirm" disabled={count === 0} onclick={onConfirm}>
    {t("assetBrowser.pickConfirm")}
  </button>
</footer>

<style lang="scss">
  .pick-bar {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
    padding: 0.375rem;
    border-top: 1px solid var(--border);
    button {
      min-height: 2.25rem;
      min-width: 44px;
    }
  }
  .count {
    margin-right: auto;
    font-size: 0.85rem;
    color: var(--text-muted);
  }
</style>
