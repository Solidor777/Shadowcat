<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    ESCAPE_PRIORITY,
    escapeCoordinator,
    findTourAnchor,
    getAppContext,
    sizeClass,
    trapFocus,
  } from "@shadowcat/ui-kit";
  import type { TourController } from "./controller.svelte";
  import { placeCard, type Box } from "./placement";

  let {
    controller,
  }: {
    /** The controller whose running tour this overlay draws. */
    controller: TourController;
  } = $props();

  const { t } = getAppContext();
  const uid = $props.id();
  const titleId = `sc-tour-title-${uid}`;
  const bodyId = `sc-tour-body-${uid}`;

  /** Padding in pixels the spotlight keeps around the target's box. */
  const SPOTLIGHT_PAD = 4;
  /** Downward drag distance in pixels that dismisses the bottom sheet. */
  const SWIPE_DISMISS_PX = 64;

  // The element focused before the tour opened; focus returns to it when the overlay closes.
  const previouslyFocused = document.activeElement;

  const active = $derived(controller.active);
  const step = $derived(active === null ? null : active.tour.steps[active.stepIndex]);
  const position = $derived(active === null ? 0 : active.shown.indexOf(active.stepIndex) + 1);
  const total = $derived(active === null ? 0 : active.shown.length);
  const isFirst = $derived(position <= 1);
  const isLast = $derived(position >= total);
  const compact = $derived(sizeClass() === "compact");

  let rect = $state<Box | null>(null);
  let viewport = $state({ width: window.innerWidth, height: window.innerHeight });
  let cardWidth = $state(0);
  let cardHeight = $state(0);
  let cardEl = $state<HTMLElement | null>(null);
  let nextEl = $state<HTMLButtonElement | null>(null);
  let swipeStartY: number | null = null;

  /** Re-reads the viewport size and the target's box; the spotlight follows resize and scroll.
   * @example
   * ```
   * // private function; installed as the window resize and scroll listener
   * measure();
   * ```
   */
  function measure(): void {
    viewport = { width: window.innerWidth, height: window.innerHeight };
    const el = step === null ? null : findTourAnchor(document, step.target);
    if (el === null) {
      rect = null;
      return;
    }
    const box = el.getBoundingClientRect();
    rect = { left: box.left, top: box.top, width: box.width, height: box.height };
  }

  const cardPosition = $derived(
    step === null || compact
      ? null
      : placeCard(rect, step.placement, { width: cardWidth, height: cardHeight }, viewport),
  );

  $effect(() => {
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  });

  // One Escape path: the shared coordinator at modal priority. Escape ends the tour like Skip.
  $effect(() => escapeCoordinator.register(ESCAPE_PRIORITY.MODAL, () => controller.end()));

  $effect(() => (cardEl ? trapFocus(cardEl) : undefined));

  // Each step moves focus to its primary button, so keyboard users continue from the card.
  $effect(() => {
    void step;
    nextEl?.focus();
  });

  onDestroy(() => {
    if (previouslyFocused instanceof HTMLElement && previouslyFocused.isConnected) previouslyFocused.focus();
  });

  /** Remembers where a touch drag on the bottom sheet started.
   * @param event The pointer-down event on the card.
   * @example
   * ```
   * // private handler; wired to the card's pointerdown
   * ```
   */
  function onPointerDown(event: PointerEvent): void {
    swipeStartY = compact && event.pointerType === "touch" ? event.clientY : null;
  }

  /** Ends the tour when a touch drag on the bottom sheet moved far enough downward.
   * @param event The pointer-up event on the card.
   * @example
   * ```
   * // private handler; wired to the card's pointerup
   * ```
   */
  function onPointerUp(event: PointerEvent): void {
    if (swipeStartY !== null && event.clientY - swipeStartY > SWIPE_DISMISS_PX) controller.end();
    swipeStartY = null;
  }
</script>

{#if step !== null}
  <div class="tour" data-testid="tour-overlay">
    <!-- Inert backdrop: it swallows pointer input so the page behind the tour stays untouched.
         Without a target the backdrop itself dims the page; with one, the spotlight's shadow does. -->
    <div class="blocker" class:dim={rect === null} role="presentation"></div>
    {#if rect !== null}
      <div
        class="spotlight"
        data-testid="tour-spotlight"
        style:left="{rect.left - SPOTLIGHT_PAD}px"
        style:top="{rect.top - SPOTLIGHT_PAD}px"
        style:width="{rect.width + SPOTLIGHT_PAD * 2}px"
        style:height="{rect.height + SPOTLIGHT_PAD * 2}px"
      ></div>
    {/if}
    <div
      class="card"
      class:sheet={compact}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={bodyId}
      tabindex="-1"
      data-testid="tour-card"
      bind:this={cardEl}
      bind:clientWidth={cardWidth}
      bind:clientHeight={cardHeight}
      style:left={cardPosition === null ? undefined : `${cardPosition.left}px`}
      style:top={cardPosition === null ? undefined : `${cardPosition.top}px`}
      onpointerdown={onPointerDown}
      onpointerup={onPointerUp}
      onpointercancel={() => (swipeStartY = null)}
    >
      <h2 id={titleId}>{t(step.titleKey)}</h2>
      <p id={bodyId}>{t(step.bodyKey)}</p>
      <p class="count" data-testid="tour-count">{t("onboarding.step", { n: position, total })}</p>
      <div class="actions">
        <button type="button" data-testid="tour-skip" onclick={() => controller.end()}>{t("onboarding.skip")}</button>
        <button type="button" data-testid="tour-back" disabled={isFirst} onclick={() => controller.back()}>
          {t("onboarding.back")}
        </button>
        <button type="button" class="next" data-testid="tour-next" bind:this={nextEl} onclick={() => controller.next()}>
          {isLast ? t("onboarding.done") : t("onboarding.next")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style lang="scss">
  .tour {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-popover) + 1);
  }
  .blocker {
    position: fixed;
    inset: 0;
  }
  .blocker.dim {
    background: var(--scrim);
  }
  /* The hole: a transparent box whose enormous shadow dims everything around the target. */
  .spotlight {
    position: fixed;
    border-radius: var(--radius-1);
    box-shadow: 0 0 0 100vmax var(--scrim);
    outline: 2px solid var(--accent);
    pointer-events: none;
  }
  .card {
    position: fixed;
    box-sizing: border-box;
    width: min(22rem, calc(100vw - var(--space-3) * 2));
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
  /* Narrow screens: a bottom sheet spanning the width, dismissed by a downward swipe. */
  .card.sheet {
    left: 0;
    right: 0;
    bottom: 0;
    width: auto;
    border-radius: var(--radius-2) var(--radius-2) 0 0;
  }
  h2 {
    margin: 0;
    font-size: var(--font-size-md);
  }
  p {
    margin: 0;
  }
  .count {
    color: var(--text-muted);
    font-size: 0.875rem;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }
  button {
    min-height: 44px;
    min-width: 44px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .next {
    background: var(--accent);
    color: var(--on-accent);
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  /* The spotlight and card jump between steps by default; the move only animates for users who
   * have not asked for reduced motion. */
  @media (prefers-reduced-motion: no-preference) {
    .spotlight,
    .card:not(.sheet) {
      transition:
        left 0.2s ease,
        top 0.2s ease,
        width 0.2s ease,
        height 0.2s ease;
    }
  }
</style>
