<script lang="ts">
  import { Surface } from "@shadowcat/ui-kit";
</script>

<div class="layout">
  <div class="topbar"><Surface contract="shadowcat.surface:topbar" /></div>
  <div class="toolrail"><Surface contract="shadowcat.surface:toolrail" /></div>
  <div class="main"><Surface contract="shadowcat.surface:panel-host" /></div>
  <div class="statusbar"><Surface contract="shadowcat.surface:statusbar" /></div>
</div>

<style lang="scss">
  .layout {
    display: grid;
    height: 100vh;
    grid-template-columns: 3rem 1fr;
    grid-template-rows: 2.5rem 1fr 1.5rem;
    grid-template-areas:
      "topbar topbar"
      "toolrail main"
      "statusbar statusbar";
    background: var(--surface-base);
    color: var(--text-primary);
  }
  .topbar {
    grid-area: topbar;
    background: var(--surface-raised);
    border-bottom: 1px solid var(--border);
  }
  .toolrail {
    grid-area: toolrail;
    background: var(--surface-overlay);
    border-right: 1px solid var(--border);
  }
  .main {
    grid-area: main;
    /* Growth cap: zeroes the grid item's automatic minimum size so tall panel
     * content scrolls inside the panel host's panels instead of growing the
     * 1fr track past 100vh. Scrolling itself is owned by the panel host. */
    min-height: 0;
    overflow: hidden;
  }
  .statusbar {
    grid-area: statusbar;
    background: var(--surface-overlay);
    border-top: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 0.8rem;
  }

  /* Phone: toolrail hidden (M12b replaces it with a touch-appropriate launcher). */
  @media (max-width: 40rem) {
    .layout {
      grid-template-columns: 1fr;
      grid-template-rows: 2.5rem 1fr 1.5rem;
      grid-template-areas: "topbar" "main" "statusbar";
    }
    .toolrail {
      display: none;
    }
  }
</style>
