<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { TourController } from "./controller.svelte";

  let {
    controller,
  }: {
    /** The controller whose pending offer this toast answers. */
    controller: TourController;
  } = $props();

  const { t } = getAppContext();
</script>

<!-- A dismissible toast pinned just above the status bar. It never takes focus or blocks input:
     the only ways it goes away are its three buttons. -->
<section class="offer" data-testid="tour-offer" aria-label={t("onboarding.offer.label")}>
  <p class="message">{t("onboarding.offer.message")}</p>
  <div class="actions">
    <button type="button" class="start" data-testid="tour-offer-start" onclick={() => controller.startOffer()}>
      {t("onboarding.offer.start")}
    </button>
    <button type="button" data-testid="tour-offer-later" onclick={() => controller.dismissOffer()}>
      {t("onboarding.offer.later")}
    </button>
    <button type="button" data-testid="tour-offer-never" onclick={() => controller.disableForever()}>
      {t("onboarding.offer.never")}
    </button>
  </div>
</section>

<style lang="scss">
  .offer {
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
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  button {
    min-height: 44px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .start {
    background: var(--accent);
    color: var(--on-accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  /* Narrow screens: the toast spans the width so its three buttons wrap inside the viewport. */
  @media (max-width: 48rem) {
    .offer {
      left: var(--space-2);
      right: var(--space-2);
      max-width: none;
    }
  }
</style>
