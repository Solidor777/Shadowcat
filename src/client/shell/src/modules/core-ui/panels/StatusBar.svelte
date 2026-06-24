<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  const { role } = getAppContext();
</script>

<footer class="statusbar"><span>{role}</span></footer>

<style lang="scss">
  .statusbar {
    display: flex;
    align-items: center;
    padding: 0 var(--space-3);
    height: 100%;
  }
</style>
