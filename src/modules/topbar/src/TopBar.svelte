<script lang="ts">
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
  import LauncherMenu from "./LauncherMenu.svelte";
  import Presence from "./Presence.svelte";

  const ctx = getAppContext();
  const { world, t } = ctx;
  const compact = $derived(sizeClass() === "compact");

  // The settings panel is a registered panel; the topbar's settings entry is a
  // stable, standard-location toggle for it (no new seam).
  const SETTINGS_PANEL_ID = "settings:panel";
  /**
   * Toggles the settings panel via `AppContext.panels` — the same seam
   * `LauncherMenu.activate` uses, making this button an alternate,
   * always-visible entry point to the SAME registered panel rather than a
   * second mechanism.
   * @returns Nothing; toggles the settings panel as a side effect.
   * @example
   * ```
   * // private function; not part of the public API — wired to the settings-entry button
   * toggleSettings();
   * ```
   */
  function toggleSettings(): void {
    ctx.panels.toggle(SETTINGS_PANEL_ID);
  }
</script>

<header class="topbar" class:compact>
  <LauncherMenu />

  <!-- World title. Scene title is deferred to M12d (scene docs carry no name yet). -->
  <div class="title" data-testid="topbar-title">
    <strong class="app">{t("app.name")}</strong>
    <span class="world">{t("topbar.world", { world })}</span>
  </div>

  <div class="spacer"></div>

  <div class="presence-slot">
    <Presence />
  </div>

  <button
    type="button"
    class="settings-entry"
    data-testid="topbar-settings"
    aria-label={t("settings.tab")}
    title={t("settings.tab")}
    onclick={toggleSettings}
  >
    <span aria-hidden="true">🔧</span>
  </button>
</header>

<style lang="scss">
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-3);
    height: 100%;
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0; /* let the world label truncate rather than push the row */
    overflow: hidden;
  }
  .title .app {
    white-space: nowrap;
  }
  .world {
    color: var(--text-muted);
    font-size: 0.875rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Compact: drop the world label; the app name + launcher + presence stay. */
  .topbar.compact .world {
    display: none;
  }
  .spacer {
    flex: 1 1 auto;
  }
  .presence-slot {
    /* Presence's own `.sc-presence { overflow: hidden }` only clips silently
       once the roster outgrows the width; give the slot room to shrink
       (default flex-item min-width is `auto`, which blocks shrinking below
       content width) so badges truncate gracefully instead of overflowing
       the bar. */
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
  }
  .settings-entry {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 44px;
    min-height: 44px; /* touch target */
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }
  }
</style>
