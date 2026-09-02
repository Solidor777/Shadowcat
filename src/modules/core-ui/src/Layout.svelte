<script lang="ts">
  import { Surface, sizeClass } from "@shadowcat/ui-kit";

  // Single breakpoint axis (ui-kit `sizeClass`, 48rem) is the only source of
  // truth for compact/expanded, shared by the toolrail layout and the panel
  // host's compact switcher.
  const compact = $derived(sizeClass() === "compact");
</script>

<div class="layout" class:compact>
  <div class="topbar"><Surface contract="shadowcat.surface:topbar" /></div>
  <!-- DOM order follows compact's visual order (main before toolrail) so
       keyboard/screen-reader traversal reaches main content before tool
       controls; grid-template-areas alone govern visual placement in BOTH
       modes, so expanded (toolrail left, main right) is unaffected. -->
  <div class="main"><Surface contract="shadowcat.surface:panel-host" /></div>
  <div class="toolrail"><Surface contract="shadowcat.surface:toolrail" /></div>
  <div class="statusbar"><Surface contract="shadowcat.surface:statusbar" /></div>
</div>
<!-- App-level overlay layer (modal scrims and similar fixed-position chrome).
     Rendered OUTSIDE and AFTER the grid so contributions here are never
     clipped by the grid's overflow and stack above every region without a
     z-index war; contributions position themselves (typically fixed). -->
<Surface contract="shadowcat.surface:overlay" />

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
  /* Growth cap on BOTH items of the 1fr row: a grid row is at least as tall as its
   * tallest item's minimum contribution, so one uncapped item grows the row, the grid and
   * every sibling past 100vh — the document then scrolls, and a canvas rect measured before
   * that scroll no longer describes where the canvas is. Zeroing the automatic minimum size
   * plus a non-visible overflow is what removes an item's content from the row's minimum.
   * `Layout.test` enumerates the row's items against this cap. */
  .toolrail {
    grid-area: toolrail;
    background: var(--surface-overlay);
    border-right: 1px solid var(--border);
    /* The rail's contributions are plain stacked content, so this cell is their scroll
     * container (the compact bottom strip scrolls itself horizontally instead; its `auto`
     * row is content-sized, so nothing overflows here). Horizontal overflow is clipped, or
     * the vertical scrollbar's own width would demand a second, horizontal one. */
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .main {
    grid-area: main;
    /* Inner scrolling is owned by the panel host's panes. */
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

  /* Compact (<48rem): single column; the toolrail is a full-width bottom tool
   * strip sized by an `auto` row —
   * content-height when the GM-gated rail renders tools, otherwise the
   * hairline `border-top` below is the row's only height. */
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
