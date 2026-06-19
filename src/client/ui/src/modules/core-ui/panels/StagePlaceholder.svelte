<script lang="ts">
  import { getAppContext } from "../../../lib/appContext";
  const { t } = getAppContext();
</script>

<section class="stage"><p>{t("stage.placeholder")}</p></section>

<style lang="scss">
  .stage {
    display: grid;
    place-items: center;
    height: 100%;
    color: var(--text-muted);
  }
</style>
