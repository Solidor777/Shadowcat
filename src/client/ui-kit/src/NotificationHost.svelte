<script lang="ts">
  // Global toast/banner host — mounted once alongside `TemplateModalHost`, not scoped to any
  // particular surface. Renders `activeNotifications()`, the UI-visible counterpart to `Logger`
  // (see `AppContext.notify`). `info`-level notifications auto-dismiss UNLESS they carry an
  // action (a call-to-action is a decision the user must be able to make at their own pace) or
  // progress (an in-flight operation must stay visible until its owner dismisses it);
  // `warning`/`error` never do — these are exactly
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
      // An action-carrying notification never auto-dismisses regardless of
      // level: the action IS the user's decision to make ("Reopen windows"),
      // and a vanished toast can't be clicked.
      // A progress notification never auto-dismisses either: an in-flight operation must stay
      // visible until its owner ends it.
      if (n.level !== "info" || n.action || n.progress || scheduled.has(n.id)) continue;
      scheduled.add(n.id);
      setTimeout(() => notifications.dismiss(n.id), INFO_AUTO_DISMISS_MS);
    }
  });
</script>

<div class="sc-notify-host" aria-live="polite">
  {#each items as n (n.id)}
    <!-- Pointer-only convenience: a click anywhere on the row dismisses it (a larger touch
         target than the dismiss button). The dismiss button stays the keyboard and screen-reader
         path. A progress row ignores the click so an accidental tap cannot hide a running
         operation. -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="sc-notify sc-notify-{n.level}"
      class:sc-notify-dismissable={!n.progress}
      role="status"
      onclick={() => {
        if (!n.progress) notifications.dismiss(n.id);
      }}
    >
      <div class="sc-notify-body">
        <span class="sc-notify-message">{n.message}</span>
        {#if n.progress}
          <div
            class="sc-notify-progress"
            class:sc-notify-progress-indeterminate={n.progress.value === null}
            role="progressbar"
            aria-label={n.message}
            aria-valuemin={n.progress.value === null ? undefined : 0}
            aria-valuenow={n.progress.value === null ? undefined : n.progress.value}
            aria-valuemax={n.progress.value === null ? undefined : n.progress.max}
            aria-valuetext={n.progress.value === null ? t("notifications.progress.working") : undefined}
          >
            {#if n.progress.value !== null}
              <div
                class="sc-notify-progress-fill"
                style:width="{n.progress.max > 0 ? Math.min(100, (n.progress.value / n.progress.max) * 100) : 0}%"
              ></div>
            {/if}
          </div>
        {/if}
      </div>
      {#if n.action}
        <button
          type="button"
          class="sc-notify-action"
          onclick={(event) => {
            // The row's click handler also dismisses; stopping propagation keeps this click
            // from dismissing twice.
            event.stopPropagation();
            // The action IS the user's response to this notification — run it
            // and dismiss together, so an already-acted-on notice can't be
            // clicked twice.
            n.action!.run();
            notifications.dismiss(n.id);
          }}
        >
          {n.action.label}
        </button>
      {/if}
      <button
        type="button"
        class="sc-notify-dismiss"
        aria-label={t("notifications.dismiss")}
        onclick={(event) => {
          event.stopPropagation();
          notifications.dismiss(n.id);
        }}
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
  .sc-notify-dismissable {
    cursor: pointer;
  }
  .sc-notify-body {
    flex: 1;
    display: grid;
    gap: var(--space-1);
    min-width: 0;
  }
  .sc-notify-message {
    font-size: 0.9rem;
  }
  .sc-notify-progress {
    position: relative;
    height: 0.4rem;
    overflow: hidden;
    border-radius: var(--radius-1);
    background: var(--surface-base);
  }
  .sc-notify-progress-fill {
    height: 100%;
    background: var(--accent);
  }
  .sc-notify-progress-indeterminate::after {
    content: "";
    position: absolute;
    inset: 0 auto 0 0;
    width: 40%;
    background: var(--accent);
    animation: sc-notify-slide 1.2s ease-in-out infinite alternate;
  }
  @keyframes sc-notify-slide {
    to {
      transform: translateX(150%);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .sc-notify-progress-indeterminate::after {
      animation: none;
      width: 100%;
      opacity: 0.5;
    }
  }
  .sc-notify-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: 44px; /* touch target (mobile invariant); >=24px a11y floor */
    padding: 0 var(--space-3);
    border: 1px solid var(--accent);
    border-radius: var(--radius-1);
    background: transparent;
    color: var(--accent);
    font-size: 0.9rem;
    cursor: pointer;
    white-space: nowrap;
    &:hover {
      background: var(--surface-base);
    }
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: -2px;
    }
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
