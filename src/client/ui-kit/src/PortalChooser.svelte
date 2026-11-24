<script lang="ts">
  // Shows the portal offer the server addressed to this connection and sends the answer. The
  // client decides nothing about who may see or answer an offer: it renders what arrives (labels,
  // and a destination scene name only when the server included one) and the server refuses an
  // unauthorized or stale answer. Compact viewports get a blocking modal; expanded ones a
  // non-blocking card so the stage stays usable. Escape declines in both.
  import { SessionPausedError, type PortalOfferNotice } from "@shadowcat/core";
  import { getAppContext } from "./appContext";
  import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";
  import { rememberFocus, trapFocus } from "./focusTrap";
  import { sizeClass } from "./sizeClass.svelte";

  const ctx = getAppContext();
  const t = ctx.t;
  const uid = $props.id();
  const titleId = `sc-portal-title-${uid}`;

  // Pending offers, oldest first; a newer offer for the same token replaces its older one, the
  // server's own one-offer-per-token rule. The first entry is the one shown.
  let offers = $state<PortalOfferNotice[]>([]);
  const current = $derived<PortalOfferNotice | undefined>(offers[0]);
  const compact = $derived(sizeClass() === "compact");

  let dialogEl = $state<HTMLDivElement | null>(null);

  $effect(() =>
    ctx.portal.onOffer((offer) => {
      offers = [...offers.filter((o) => o.token !== offer.token), offer];
    }),
  );

  /**
   * Answer the shown offer and drop it from the list: an offer answers once, accepted or declined,
   * so it never stays up waiting on the reply. A refusal is shown as a notification carrying the
   * server's (role-conditioned) wording, or the paused notice when the session pause refused it.
   * @param offer - The offer being answered.
   * @param choice - The chosen option's `index`, or `null` to decline.
   * @example
   * answer(offer, 0);
   */
  function answer(offer: PortalOfferNotice, choice: number | null): void {
    offers = offers.filter((o) => o.offerId !== offer.offerId);
    const sent = choice === null ? ctx.portal.decline(offer.offerId) : ctx.portal.choose(offer.offerId, choice);
    sent.catch((e: unknown) => {
      ctx.notify(e instanceof SessionPausedError ? t("intent.rejected.paused") : e instanceof Error ? e.message : String(e), "error");
    });
  }

  // One Escape path: the shared coordinator, registered only while an offer is shown so an idle
  // host never consumes an Escape aimed elsewhere. The compact modal blocks the page and ranks as
  // a modal; the wide card does not block, so it takes the lowest band and an Escape aimed at an
  // open menu, tooltip or overlay dismisses that first instead of declining the portal.
  $effect(() => {
    const offer = current;
    if (offer === undefined) return undefined;
    const priority = compact ? ESCAPE_PRIORITY.MODAL : ESCAPE_PRIORITY.TOOL;
    return escapeCoordinator.register(priority, () => answer(offer, null));
  });

  // The compact modal traps Tab; the wide card does not, but both return focus to the opener.
  $effect(() => {
    if (!dialogEl) return undefined;
    return compact ? trapFocus(dialogEl) : rememberFocus(dialogEl);
  });

  // Focus the first available option whenever a different offer is shown.
  $effect(() => {
    if (current === undefined) return;
    dialogEl?.querySelector<HTMLButtonElement>("button.option:not([disabled])")?.focus();
  });

  /**
   * Move focus between the option buttons with the arrow keys; Enter activates the focused button
   * natively.
   * @param event - The keydown event on the dialog.
   * @example
   * onKeydown(new KeyboardEvent("keydown", { key: "ArrowDown" }));
   */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const buttons = Array.from(dialogEl?.querySelectorAll<HTMLButtonElement>("button.option:not([disabled])") ?? []);
    if (buttons.length === 0) return;
    event.preventDefault();
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const step = event.key === "ArrowDown" ? 1 : -1;
    buttons[(at + step + buttons.length) % buttons.length].focus();
  }
</script>

{#if current}
  {@const offer = current}
  <div class="sc-portal-scrim" class:compact role="presentation">
    <div
      class="sc-portal"
      role="dialog"
      aria-modal={compact ? "true" : undefined}
      aria-labelledby={titleId}
      tabindex="-1"
      data-testid="portal-chooser"
      data-layout={compact ? "modal" : "card"}
      bind:this={dialogEl}
      onkeydown={onKeydown}
    >
      <h2 id={titleId}>{offer.prompt ?? t("portal.title")}</h2>
      <div class="options">
        {#each offer.options as option (option.index)}
          <button
            type="button"
            class="option"
            data-testid="portal-option"
            disabled={!option.available}
            onclick={() => answer(offer, option.index)}
          >
            <span class="label">{option.label}</span>
            {#if option.destination_name !== null}
              <span class="destination" data-testid="portal-destination">{option.destination_name}</span>
            {/if}
            {#if !option.available}
              <span class="destination">{t("portal.unavailable")}</span>
            {/if}
          </button>
        {/each}
      </div>
      <div class="actions">
        {#if offers.length > 1}
          <span class="more">{t("portal.more", { count: offers.length - 1 })}</span>
        {/if}
        <button type="button" data-testid="portal-decline" onclick={() => answer(offer, null)}>{t("portal.decline")}</button>
      </div>
    </div>
  </div>
{/if}

<style lang="scss">
  // Expanded: the wrapper is a pointer-transparent anchor so only the card takes clicks and the
  // stage underneath stays usable. Compact: a scrim blocks the stage behind a centered modal.
  .sc-portal-scrim {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    padding: var(--space-3);
    pointer-events: none;
    z-index: var(--z-popover);
  }
  .sc-portal-scrim.compact {
    align-items: center;
    background: var(--scrim);
    pointer-events: auto;
  }
  .sc-portal {
    pointer-events: auto;
    background: var(--surface-raised);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    padding: var(--space-3);
    width: min(90vw, 24rem);
    max-height: 85vh;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--font-size-md);
  }
  .options {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
  }
  .more {
    margin-right: auto;
    opacity: 0.7;
  }
  .destination {
    display: block;
    opacity: 0.7;
    font-size: var(--font-size-sm);
  }
  .label {
    display: block;
    font-weight: 600;
  }
  // Touch-target floor (mobile invariant).
  button {
    min-height: 44px;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: inherit;
    text-align: left;
  }
  .actions button {
    text-align: center;
  }
  .option:disabled {
    opacity: 0.5;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
