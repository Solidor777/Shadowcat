<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { TourController } from "./controller.svelte";

  let {
    controller,
  }: {
    /** The controller whose instructions this toast shows. */
    controller: TourController;
  } = $props();

  const { t } = getAppContext();
</script>

<!-- Tells the user how to start a tour later. It follows every answer to the offer, takes no
     focus, and goes away only through its button. -->
<section class="hint" data-testid="tour-hint" aria-label={t("onboarding.offer.label")}>
  <p class="message">{t("onboarding.hint.message")}</p>
  <button type="button" data-testid="tour-hint-dismiss" onclick={() => controller.acknowledgeHowTo()}>
    {t("onboarding.hint.dismiss")}
  </button>
</section>

<style lang="scss">
  .hint {
    position: fixed;
    left: var(--space-3);
    bottom: calc(var(--statusbar-height) + var(--space-2));
    max-width: min(24rem, calc(100vw - var(--space-3) * 2));
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--surface-raised);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    box-shadow: var(--shadow-elevated);
  }
  .message {
    margin: 0;
  }
  button {
    align-self: flex-start;
    min-height: 44px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--accent);
    color: var(--on-accent);
    font: inherit;
    cursor: pointer;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  /* Narrow screens: the toast spans the width, as the offer does. */
  @media (max-width: 48rem) {
    .hint {
      left: var(--space-2);
      right: var(--space-2);
      max-width: none;
    }
  }
</style>
