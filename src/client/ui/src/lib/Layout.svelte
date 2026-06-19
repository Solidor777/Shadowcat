<script lang="ts">
  import Surface from "./Surface.svelte";
</script>

<div class="layout">
  <div class="topbar"><Surface contract="shadowcat.surface:topbar" /></div>
  <div class="toolrail"><Surface contract="shadowcat.surface:toolrail" /></div>
  <div class="stage"><Surface contract="shadowcat.surface:stage" /></div>
  <div class="sidebar"><Surface contract="shadowcat.surface:sidebar" /></div>
  <div class="statusbar"><Surface contract="shadowcat.surface:statusbar" /></div>
</div>

<style>
  .layout {
    display: grid;
    height: 100vh;
    grid-template-columns: 3rem 1fr 20rem;
    grid-template-rows: 2.5rem 1fr 1.5rem;
    grid-template-areas:
      "topbar topbar sidebar"
      "toolrail stage sidebar"
      "statusbar statusbar sidebar";
  }
  .topbar { grid-area: topbar; }
  .toolrail { grid-area: toolrail; }
  .stage { grid-area: stage; }
  .sidebar { grid-area: sidebar; overflow: auto; }
  .statusbar { grid-area: statusbar; }

  /* Phone: stack regions, sidebar below the stage. */
  @media (max-width: 40rem) {
    .layout {
      grid-template-columns: 1fr;
      grid-template-rows: 2.5rem 1fr auto 1.5rem;
      grid-template-areas: "topbar" "stage" "sidebar" "statusbar";
    }
    .toolrail { display: none; }
  }
</style>
