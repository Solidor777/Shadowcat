<script lang="ts">
  import { Surface, sizeClass } from "@shadowcat/ui-kit";

  // Single breakpoint axis (ui-kit `sizeClass`, 48rem) — the only source of
  // truth for compact/expanded. Replaces the removed 40rem media query so the
  // toolrail-hide threshold and the panel host's compact switcher flip together.
  const compact = $derived(sizeClass() === "compact");
</script>

<div class="layout" class:compact>
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
    grid-template-rows: 2.5rem 1fr 2rem;
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
     * content scrolls inside the panel host's panes instead of growing the 1fr
     * track past 100vh. Inner scrolling is owned by the panel host. */
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

  /* Compact (<48rem): single column; the toolrail becomes a full-width bottom
   * tool strip (an `auto` row that collapses to 0 when the GM-gated rail renders
   * nothing) instead of being hidden — real mobile tooling per spec §4.4/§8. */
  .layout.compact {
    grid-template-columns: 1fr;
    grid-template-rows: 2.5rem 1fr auto 2rem;
    grid-template-areas:
      "topbar"
      "main"
      "toolrail"
      "statusbar";
  }
  .layout.compact .toolrail {
    border-right: none;
    border-top: 1px solid var(--border);
  }
</style>
