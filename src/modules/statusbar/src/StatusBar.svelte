<script lang="ts">
  import { getAppContext, Surface } from "@shadowcat/ui-kit";
  import PerfStats from "./PerfStats.svelte";
  const ctx = getAppContext();
  const { role } = ctx;
  let unlocked = $state(false);

  /** Unlock this device's `AudioContext` from the click gesture, then swap to the mute toggle.
   * @example
   * ```
   * // component handler; exercised through `StatusBar.test.ts`'s unlock-click case
   * ```
   */
  async function unlock(): Promise<void> {
    await ctx.audio.unlock();
    unlocked = true;
  }
</script>

<footer class="statusbar">
  <span>{role}</span>
  {#if !unlocked}
    <button type="button" data-testid="audio-unlock" onclick={unlock}>{ctx.t("statusbar.audio.enable")}</button>
  {:else}
    <button
      type="button"
      data-testid="audio-mute-toggle"
      aria-label={ctx.t(ctx.audio.channels.master.muted ? "statusbar.audio.unmute" : "statusbar.audio.mute")}
      onclick={() => ctx.audio.setChannel("master", { muted: !ctx.audio.channels.master.muted })}
    >{ctx.audio.channels.master.muted ? "🔇" : "🔊"}</button>
  {/if}
  <PerfStats />
  <div class="dock"><Surface contract="shadowcat.surface:panel-dock" /></div>
</footer>

<style lang="scss">
  .statusbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
    height: 100%;
  }
  /* The row is a fixed `2rem` grid track (`Layout.svelte`); unlike the toolrail's `1fr` row
   * it carries no growth cap, so a button left at UA default sizing (unset font/padding/border)
   * renders a few px taller than the track and overflows the grid past 100vh. Inheriting the
   * row's own font-size and using compact padding keeps every statusbar button inside it. */
  .statusbar button {
    font: inherit;
    line-height: 1;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: inherit;
    cursor: pointer;
  }
  .dock {
    margin-left: auto;
  }
</style>
