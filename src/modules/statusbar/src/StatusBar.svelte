<script lang="ts">
  import { getAppContext, Surface } from "@shadowcat/ui-kit";
  const { role } = getAppContext();
</script>

<footer class="statusbar">
  <span>{role}</span>
  <div class="dock"><Surface contract="shadowcat.surface:panel-dock" /></div>
</footer>

<style lang="scss">
  .statusbar {
    display: flex;
    align-items: center;
    padding: 0 var(--space-3);
    height: 100%;
  }
  .dock {
    margin-left: auto;
  }
</style>
