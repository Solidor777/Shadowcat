<script lang="ts">
  // Global toast/banner host — mounted once alongside `TemplateModalHost`, not scoped to any
  // particular surface. Renders `activeNotifications()`, the UI-visible counterpart to `Logger`
  // (see `AppContext.notify`). `info`-level notifications auto-dismiss (they're transient status
  // confirmations a user doesn't need to act on); `warning`/`error` never do — these are exactly
  // the class of message ("an operation partially applied") a user must not miss by looking away.
  import { notifications, activeNotifications } from "./notifications.svelte";
  import { t } from "./i18n.svelte";

  /** How long an `info`-level notification stays before auto-dismissing. */
  const INFO_AUTO_DISMISS_MS = 5000;

  const items = $derived(activeNotifications());

  // Tracks which ids already have an auto-dismiss timer scheduled, so a re-render (a sibling
  // notification pushed/dismissed) never re-schedules the same `info` item's timer.
  const scheduled = new Set<string>();

  $effect(() => {
    for (const n of items) {
      if (n.level !== "info" || scheduled.has(n.id)) continue;
      scheduled.add(n.id);
      setTimeout(() => notifications.dismiss(n.id), INFO_AUTO_DISMISS_MS);
    }
  });
</script>

<div class="sc-notify-host" aria-live="polite">
  {#each items as n (n.id)}
    <div class="sc-notify sc-notify-{n.level}" role="status">
      <span class="sc-notify-message">{n.message}</span>
      <button
        type="button"
        class="sc-notify-dismiss"
        aria-label={t("notifications.dismiss")}
        onclick={() => notifications.dismiss(n.id)}
      >
        ✕
      </button>
    </div>
  {/each}
</div>

<style lang="scss">
  .sc-notify-host {
    position: fixed;
    top: var(--space-4);
    right: var(--space-4);
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 24rem;
    pointer-events: none;
  }
  .sc-notify {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-left: 4px solid var(--accent);
    border-radius: var(--radius-2);
    background: var(--surface-overlay);
    color: var(--text-primary);
    box-shadow: var(--shadow-elevated);
    pointer-events: auto;
  }
  .sc-notify-warning {
    border-left-color: var(--warning);
  }
  .sc-notify-error {
    border-left-color: var(--danger);
  }
  .sc-notify-message {
    flex: 1;
    font-size: 0.9rem;
  }
  .sc-notify-dismiss {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 44px; /* touch target (mobile invariant); >=24px a11y floor */
    min-height: 44px;
    border: none;
    border-radius: var(--radius-1);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    &:hover {
      background: var(--surface-base);
    }
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
  }
</style>
