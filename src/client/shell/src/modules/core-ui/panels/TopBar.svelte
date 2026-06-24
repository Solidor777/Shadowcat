<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  const { world, t } = getAppContext();
</script>

<header class="topbar"><strong>{t("app.name")}</strong> <span>{t("topbar.world", { world })}</span></header>

<style lang="scss">
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-3);
    height: 100%;
  }
  .topbar span {
    color: var(--text-muted);
    font-size: 0.875rem;
  }
</style>
