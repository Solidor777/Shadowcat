<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import type { TourMeta } from "@shadowcat/core";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { TourController } from "./controller.svelte";
  import { matchesAudience } from "./eligibility";
  import { registeredTours, viewerOf } from "./environment";

  let {
    controller,
  }: {
    /** The session's tour controller; Replay starts a tour through it. */
    controller: TourController;
  } = $props();

  const ctx = getAppContext();
  const { t } = ctx;

  // Bridges the registry's subscribe/snapshot to Svelte's reactivity, the same pattern
  // `Settings.svelte` uses for the contribution registry.
  const subscribeTours = createSubscriber((update) => ctx.contributions.subscribe(update));
  const tours = $derived.by(() => {
    subscribeTours();
    const viewer = viewerOf(ctx);
    return registeredTours(ctx.contributions).filter((tour) => matchesAudience(tour.audience, viewer));
  });

  /** Starts `tour`; tells the user when none of its steps is on screen.
   * @param tour The tour to replay.
   * @example
   * ```
   * // private handler; wired to each row's Replay button
   * ```
   */
  function replay(tour: TourMeta): void {
    if (!controller.start(tour)) ctx.notify(t("onboarding.replay.unavailable"), "info");
  }
</script>

<ul class="tours" data-testid="tour-replay-list">
  {#each tours as tour (tour.id)}
    <li>
      <span class="name">{t(tour.labelKey)}</span>
      <button type="button" data-testid={`tour-replay-${tour.id}`} onclick={() => replay(tour)}>
        {t("onboarding.replay.button")}
      </button>
    </li>
  {:else}
    <li class="empty">{t("onboarding.replay.empty")}</li>
  {/each}
</ul>

<style lang="scss">
  .tours {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .name {
    flex: 1 1 8rem;
  }
  .empty {
    color: var(--text-muted);
  }
  @media (pointer: coarse) {
    button {
      min-height: var(--input-height-coarse);
    }
  }
</style>
